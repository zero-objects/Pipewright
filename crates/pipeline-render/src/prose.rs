//! Format-neutral prose model for the runbook: the pipeline described in full,
//! consistent sentences with inline code spans (commands, values, images,
//! paths) and numbered command insets. One [`describe`] pass builds the model;
//! the three renderers ([`to_markdown`], [`to_html_body`], [`to_rtf_body`])
//! turn it into Markdown / HTML / RTF so every export reads identically.

use std::fmt::Write as _;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::model::{Job, Pipeline};

/// An inline run within a sentence.
#[derive(Clone)]
pub enum Span {
    Text(String),
    /// `monospace` — a command, value, image, or path.
    Code(String),
    /// **bold** — a job name.
    Strong(String),
    /// _italic_ — a stage name.
    Em(String),
}

/// A block in a section body.
#[derive(Clone)]
pub enum Block {
    /// A paragraph of prose.
    Para(Vec<Span>),
    /// A numbered inset of command lines.
    Commands(Vec<String>),
}

/// One titled section: a job described in prose.
pub struct Section {
    pub title: String,
    pub blocks: Vec<Block>,
}

/// The whole runbook as prose: an overview followed by one section per job.
pub struct Prose {
    pub overview: Vec<Block>,
    pub sections: Vec<Section>,
}

/// The user-editable runbook wording for ONE locale, loaded from
/// `catalog/prose/<locale>.toml`. Sentence shapes in `[pipeline]`/`[job]`; every
/// locale-varying atom (pluralisation, conjunction, clause fragments) in
/// `[grammar]` — so a new language is a pure-data file.
#[derive(Deserialize)]
struct Templates {
    pipeline: PipelineTpl,
    job: JobTpl,
    grammar: GrammarTpl,
}
#[derive(Deserialize)]
struct PipelineTpl {
    named: String,
    unnamed: String,
}
#[derive(Deserialize)]
struct JobTpl {
    context: String,
    one_step: String,
    many_intro: String,
    no_steps: String,
}
#[derive(Deserialize)]
struct GrammarTpl {
    job_singular: String,
    job_plural: String,
    conj_and: String,
    conj_comma: String,
    across_stages: String,
    triggered_by: String,
    runs: String,
    runs_in_stage: String,
    runs_in_container: String,
    runs_in_stage_and_container: String,
    needs_one: String,
    needs_many: String,
    condition: String,
    params_lead: String,
    env_one: String,
    env_many: String,
    artifacts: String,
    set_other: String,
    sentence_end: String,
}

/// The locales whose prose is embedded at build time. Add a `catalog/prose/<x>.toml`
/// and one line here to ship a new language.
const LOCALES: &[(&str, &str)] = &[
    ("en", include_str!("../../../catalog/prose/en.toml")),
    ("de", include_str!("../../../catalog/prose/de.toml")),
];

/// The default locale used by [`describe`] and the render entry points.
pub const DEFAULT_LOCALE: &str = "en";

/// The templates for `locale`, parsed once and memoised. Falls back to
/// [`DEFAULT_LOCALE`] for an unknown locale.
fn templates_for(locale: &str) -> &'static Templates {
    static CACHE: OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, &'static Templates>>,
    > = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut map = cache.lock().expect("prose locale cache");
    if let Some(t) = map.get(locale) {
        return t;
    }
    let src = LOCALES
        .iter()
        .find(|(l, _)| *l == locale)
        .or_else(|| LOCALES.iter().find(|(l, _)| *l == DEFAULT_LOCALE))
        .map(|(_, s)| *s)
        .expect("default locale embedded");
    let parsed: &'static Templates = Box::leak(Box::new(
        toml::from_str(src).expect("catalog/prose/<locale>.toml parse"),
    ));
    map.insert(locale.to_string(), parsed);
    parsed
}

/// Fill a template string: literal runs become `Text` spans; each `{slot}` is
/// replaced by the spans bound to it (an absent / empty slot contributes
/// nothing). The connective wording is the template's; the grammar is in the
/// slot values.
fn fill(tmpl: &str, slots: &[(&str, Vec<Span>)]) -> Vec<Span> {
    let mut out = Vec::new();
    let mut lit = String::new();
    let mut chars = tmpl.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            if !lit.is_empty() {
                out.push(Span::Text(std::mem::take(&mut lit)));
            }
            let mut name = String::new();
            for c in chars.by_ref() {
                if c == '}' {
                    break;
                }
                name.push(c);
            }
            if let Some((_, spans)) = slots.iter().find(|(k, _)| *k == name) {
                out.extend(spans.iter().cloned());
            }
        } else {
            lit.push(c);
        }
    }
    if !lit.is_empty() {
        out.push(Span::Text(lit));
    }
    out
}

/// Build the prose description of a pipeline in the [`DEFAULT_LOCALE`].
#[must_use]
pub fn describe(p: &Pipeline) -> Prose {
    describe_in(p, DEFAULT_LOCALE)
}

/// Build the prose description of a pipeline in `locale` (all wording + grammar
/// from `catalog/prose/<locale>.toml`; falls back to the default locale if
/// unknown).
#[must_use]
pub fn describe_in(p: &Pipeline, locale: &str) -> Prose {
    let t = templates_for(locale);
    Prose {
        overview: overview(p, t),
        sections: p.jobs.iter().map(|j| describe_job(j, t)).collect(),
    }
}

fn overview(p: &Pipeline, t: &Templates) -> Vec<Block> {
    let g = &t.grammar;
    let n = p.jobs.len();
    let jobs_word = if n == 1 {
        &g.job_singular
    } else {
        &g.job_plural
    };

    let stages = if p.stages.is_empty() {
        vec![]
    } else {
        fill(
            &g.across_stages,
            &[("stages", joined(&p.stages, Span::Em, g))],
        )
    };
    let triggers = if p.triggers.is_empty() {
        vec![]
    } else {
        fill(
            &g.triggered_by,
            &[("triggers", joined(&p.triggers, Span::Text, g))],
        )
    };
    let (tmpl, name) = match &p.name {
        Some(name) => (&t.pipeline.named, vec![Span::Strong(name.clone())]),
        None => (&t.pipeline.unnamed, vec![]),
    };
    let spans = fill(
        tmpl,
        &[
            ("name", name),
            ("count", vec![Span::Text(n.to_string())]),
            ("jobs_word", vec![Span::Text(jobs_word.clone())]),
            ("stages", stages),
            ("triggers", triggers),
        ],
    );
    vec![Block::Para(spans)]
}

fn describe_job(j: &Job, t: &Templates) -> Section {
    let mut blocks = Vec::new();

    // Sentence 1: built from the `job.context` template + grammar-filled slots.
    let context = fill(
        &t.job.context,
        &[
            ("name", vec![Span::Strong(j.name.clone())]),
            ("verb", verb_clause(j, t)),
            ("needs", needs_clause(j, t)),
            ("condition", condition_clause(j, t)),
        ],
    );
    blocks.push(Block::Para(context));

    // Sentence 2: notable parameters (env, artifacts, other scalars) in prose.
    if let Some(para) = params_sentence(j, t) {
        blocks.push(Block::Para(para));
    }

    // Steps.
    match j.steps.len() {
        0 => blocks.push(Block::Para(fill(&t.job.no_steps, &[]))),
        1 => blocks.push(Block::Para(fill(
            &t.job.one_step,
            &[("command", vec![Span::Code(j.steps[0].label.clone())])],
        ))),
        _ => {
            blocks.push(Block::Para(fill(&t.job.many_intro, &[])));
            blocks.push(Block::Commands(
                j.steps.iter().map(|st| st.label.clone()).collect(),
            ));
        }
    }

    Section {
        title: j.name.clone(),
        blocks,
    }
}

/// The `{verb}` slot: where the job runs (stage and/or container) — fragment +
/// `{stage}`/`{image}` slots from the locale grammar.
fn verb_clause(j: &Job, t: &Templates) -> Vec<Span> {
    let g = &t.grammar;
    let image = j
        .params
        .iter()
        .find(|p| p.key == "image")
        .map(|p| p.value.clone());
    match (&j.stage, &image) {
        (Some(st), Some(img)) => fill(
            &g.runs_in_stage_and_container,
            &[
                ("stage", vec![Span::Em(st.clone())]),
                ("image", vec![Span::Code(img.clone())]),
            ],
        ),
        (Some(st), None) => fill(&g.runs_in_stage, &[("stage", vec![Span::Em(st.clone())])]),
        (None, Some(img)) => fill(
            &g.runs_in_container,
            &[("image", vec![Span::Code(img.clone())])],
        ),
        (None, None) => fill(&g.runs, &[]),
    }
}

/// The `{needs}` slot: "after **a** and **b** complete" (or empty).
fn needs_clause(j: &Job, t: &Templates) -> Vec<Span> {
    if j.needs.is_empty() {
        return vec![];
    }
    let g = &t.grammar;
    let tmpl = if j.needs.len() == 1 {
        &g.needs_one
    } else {
        &g.needs_many
    };
    fill(tmpl, &[("jobs", joined(&j.needs, Span::Strong, g))])
}

/// The `{condition}` slot (or empty).
fn condition_clause(j: &Job, t: &Templates) -> Vec<Span> {
    match &j.condition {
        Some(c) => fill(
            &t.grammar.condition,
            &[("expr", vec![Span::Code(c.clone())])],
        ),
        None => vec![],
    }
}

/// A sentence about a job's environment, artifacts, and other scalar params.
fn params_sentence(j: &Job, t: &Templates) -> Option<Vec<Span>> {
    let g = &t.grammar;
    let envs: Vec<String> = j
        .params
        .iter()
        .filter(|p| p.key == "env")
        .map(|p| p.value.clone())
        .collect();
    let arts: Vec<&str> = j
        .params
        .iter()
        .filter(|p| p.key == "artifacts")
        .map(|p| p.value.as_str())
        .collect();
    let others: Vec<(&str, &str)> = j
        .params
        .iter()
        .filter(|p| !matches!(p.key.as_str(), "image" | "env" | "artifacts"))
        .map(|p| (p.key.as_str(), p.value.as_str()))
        .collect();
    if envs.is_empty() && arts.is_empty() && others.is_empty() {
        return None;
    }

    let mut clauses: Vec<Vec<Span>> = Vec::new();
    if !envs.is_empty() {
        let tmpl = if envs.len() == 1 {
            &g.env_one
        } else {
            &g.env_many
        };
        clauses.push(fill(tmpl, &[("vars", joined(&envs, Span::Code, g))]));
    }
    for a in &arts {
        clauses.push(fill(
            &g.artifacts,
            &[("paths", vec![Span::Code((*a).to_string())])],
        ));
    }
    for (k, v) in &others {
        clauses.push(fill(
            &g.set_other,
            &[
                ("key", vec![Span::Text((*k).to_string())]),
                ("value", vec![Span::Code((*v).to_string())]),
            ],
        ));
    }

    let mut s = fill(&g.params_lead, &[]);
    let last = clauses.len() - 1;
    for (i, c) in clauses.into_iter().enumerate() {
        if i > 0 {
            s.extend(conj(i == last, g));
        }
        s.extend(c);
    }
    s.extend(fill(&g.sentence_end, &[]));
    Some(s)
}

/// The "a, b and c" run as spans (each item wrapped; connective from grammar).
fn joined(items: &[String], wrap: fn(String) -> Span, g: &GrammarTpl) -> Vec<Span> {
    let mut out = Vec::new();
    let last = items.len().saturating_sub(1);
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            out.extend(conj(i == last, g));
        }
        out.push(wrap(it.clone()));
    }
    out
}

/// The connective before an item: the `and` form before the last, else the comma.
fn conj(is_last: bool, g: &GrammarTpl) -> Vec<Span> {
    vec![Span::Text(if is_last {
        g.conj_and.clone()
    } else {
        g.conj_comma.clone()
    })]
}

// --- renderers -------------------------------------------------------------

/// The whole runbook as Markdown.
#[must_use]
pub fn to_markdown(pr: &Prose) -> String {
    let mut s = String::new();
    for b in &pr.overview {
        s.push_str(&md_block(b));
    }
    for sec in &pr.sections {
        let _ = write!(s, "\n## {}\n\n", sec.title);
        for b in &sec.blocks {
            s.push_str(&md_block(b));
        }
    }
    s
}

/// The runbook body as an HTML fragment (overview + per-job sections).
#[must_use]
pub fn to_html_body(pr: &Prose) -> String {
    let mut s = String::new();
    for b in &pr.overview {
        s.push_str(&html_block(b));
    }
    for sec in &pr.sections {
        let _ = write!(s, "<section class=\"job\"><h3>{}</h3>", esc(&sec.title));
        for b in &sec.blocks {
            s.push_str(&html_block(b));
        }
        s.push_str("</section>");
    }
    s
}

/// The blocks of one section as an HTML fragment (no title).
#[must_use]
pub fn section_html(blocks: &[Block]) -> String {
    blocks.iter().map(html_block).collect()
}

/// The runbook body as RTF paragraphs.
#[must_use]
pub fn to_rtf_body(pr: &Prose) -> String {
    let mut s = String::new();
    for b in &pr.overview {
        s.push_str(&rtf_block(b));
    }
    for sec in &pr.sections {
        let _ = write!(s, "\\par {{\\b\\fs26 {}}}\\par ", rtf_esc(&sec.title));
        for b in &sec.blocks {
            s.push_str(&rtf_block(b));
        }
    }
    s
}

fn md_block(b: &Block) -> String {
    match b {
        Block::Para(spans) => format!("{}\n\n", spans.iter().map(md_span).collect::<String>()),
        Block::Commands(cmds) => {
            let mut s = String::new();
            for c in cmds {
                let _ = writeln!(s, "1. `{}`", c.replace('`', "'"));
            }
            s.push('\n');
            s
        }
    }
}

fn md_span(s: &Span) -> String {
    match s {
        Span::Text(t) => t.clone(),
        Span::Code(t) => format!("`{}`", t.replace('`', "'")),
        Span::Strong(t) => format!("**{t}**"),
        Span::Em(t) => format!("_{t}_"),
    }
}

fn html_block(b: &Block) -> String {
    match b {
        Block::Para(spans) => format!("<p>{}</p>", spans.iter().map(html_span).collect::<String>()),
        Block::Commands(cmds) => {
            let mut items = String::new();
            for c in cmds {
                let _ = write!(items, "<li><code>{}</code></li>", esc(c));
            }
            format!("<ol class=\"steps\">{items}</ol>")
        }
    }
}

fn html_span(s: &Span) -> String {
    match s {
        Span::Text(t) => esc(t),
        Span::Code(t) => format!("<code>{}</code>", esc(t)),
        Span::Strong(t) => format!("<strong>{}</strong>", esc(t)),
        Span::Em(t) => format!("<em>{}</em>", esc(t)),
    }
}

fn rtf_block(b: &Block) -> String {
    match b {
        Block::Para(spans) => format!("{}\\par ", spans.iter().map(rtf_span).collect::<String>()),
        Block::Commands(cmds) => {
            let mut s = String::new();
            for (i, c) in cmds.iter().enumerate() {
                let _ = write!(s, "{}. {{\\f1 {}}}\\par ", i + 1, rtf_esc(c));
            }
            s
        }
    }
}

fn rtf_span(s: &Span) -> String {
    match s {
        Span::Text(t) => rtf_esc(t),
        Span::Code(t) => format!("{{\\f1 {}}}", rtf_esc(t)),
        Span::Strong(t) => format!("{{\\b {}}}", rtf_esc(t)),
        Span::Em(t) => format!("{{\\i {}}}", rtf_esc(t)),
    }
}

/// Minimal XML/HTML text escaping.
pub(crate) fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Escape text for RTF: control chars `\ { }` and any non-ASCII as `\uN?`.
pub(crate) fn rtf_esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            c if (c as u32) < 128 => out.push(c),
            c => {
                let n = c as u32;
                if n <= 0x7fff {
                    let _ = write!(out, "\\u{n}?");
                } else if n <= 0xffff {
                    let _ = write!(out, "\\u{}?", i32::try_from(n).unwrap_or(0) - 0x1_0000);
                } else {
                    out.push('?');
                }
            }
        }
    }
    out
}
