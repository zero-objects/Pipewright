//! The human-readable runbook: the pipeline described in full, consistent
//! sentences (with inline code spans and command insets), projected from the
//! IR. The prose model lives in [`crate::prose`]; this module wraps it into the
//! public surfaces — a standalone HTML document, Markdown, RTF, and the
//! structured `{overview, toc, jobs, skipped}` the UI consumes.
//!
//! Conceptually this is just another emit target: IR → a natural-language
//! surface, the same way the platform emitters render IR → YAML.

use std::fmt::Write as _;

use crate::model::{Job, Pipeline};
use crate::prose::{self, describe_in, DEFAULT_LOCALE};

/// Render the pipeline as a standalone, styled HTML runbook (default locale).
#[must_use]
pub fn html(p: &Pipeline) -> String {
    html_in(p, DEFAULT_LOCALE)
}

/// Render the HTML runbook in `locale` (see `catalog/prose/<locale>.toml`).
#[must_use]
pub fn html_in(p: &Pipeline, locale: &str) -> String {
    let title = p.name.as_deref().unwrap_or("Pipeline");
    let mut s = String::from("<!doctype html><meta charset=\"utf-8\">");
    s.push_str(HTML_STYLE);
    let _ = write!(
        s,
        "<article class=\"runbook\"><h1>{}</h1>",
        prose::esc(title)
    );
    s.push_str(&prose::to_html_body(&describe_in(p, locale)));
    s.push_str("</article>");
    s
}

/// Render the pipeline as Markdown (default locale).
#[must_use]
pub fn markdown(p: &Pipeline) -> String {
    markdown_in(p, DEFAULT_LOCALE)
}

/// Render Markdown in `locale`.
#[must_use]
pub fn markdown_in(p: &Pipeline, locale: &str) -> String {
    let title = p.name.as_deref().unwrap_or("Pipeline");
    format!(
        "# {title}\n\n{}",
        prose::to_markdown(&describe_in(p, locale))
    )
}

/// Render the pipeline as RTF — a Word/Pages/LibreOffice-openable document.
#[must_use]
pub fn rtf(p: &Pipeline) -> String {
    rtf_in(p, DEFAULT_LOCALE)
}

/// Render RTF in `locale`.
#[must_use]
pub fn rtf_in(p: &Pipeline, locale: &str) -> String {
    let mut s = String::from("{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Helvetica;}{\\f1 Menlo;}}\n");
    let _ = write!(
        s,
        "{{\\b\\fs36 {}}}\\par \\par ",
        prose::rtf_esc(p.name.as_deref().unwrap_or("Pipeline"))
    );
    s.push_str(&prose::to_rtf_body(&describe_in(p, locale)));
    s.push('}');
    s
}

/// Export the runbook in `format` ("md" | "html" | "doc"/"rtf"), default locale.
#[must_use]
pub fn export(p: &Pipeline, format: &str) -> Option<(String, &'static str)> {
    export_in(p, format, DEFAULT_LOCALE)
}

/// Export the runbook in `format` and `locale`. Returns the content and the
/// file extension to suggest, or `None` for an unknown format.
#[must_use]
pub fn export_in(p: &Pipeline, format: &str, locale: &str) -> Option<(String, &'static str)> {
    match format {
        "md" | "markdown" => Some((markdown_in(p, locale), "md")),
        "html" => Some((html_in(p, locale), "html")),
        "doc" | "rtf" => Some((rtf_in(p, locale), "rtf")),
        _ => None,
    }
}

/// The structured runbook the UI's human view consumes: `{overview, toc, jobs,
/// skipped}`. `overview` is the overview prose as HTML; each job becomes a `toc`
/// entry (title + one-line summary) and a `jobs` section (title + prose HTML).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Runbook {
    pub overview: String,
    pub toc: Vec<TocEntry>,
    pub jobs: Vec<JobSection>,
    /// HTML note about jobs absent from the IR; empty when nothing was dropped.
    pub skipped: String,
}

/// One table-of-contents entry: a job title and a terse one-line summary.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TocEntry {
    pub title: String,
    pub summary: String,
    pub anchor: String,
}

/// One runbook section: a job's title and its prose body as HTML.
#[derive(Debug, Clone, serde::Serialize)]
pub struct JobSection {
    pub anchor: String,
    pub title: String,
    pub html: String,
}

/// Build the structured [`Runbook`] for the UI (default locale).
#[must_use]
pub fn runbook(p: &Pipeline) -> Runbook {
    runbook_in(p, DEFAULT_LOCALE)
}

/// Build the structured [`Runbook`] for the UI in `locale`.
#[must_use]
pub fn runbook_in(p: &Pipeline, locale: &str) -> Runbook {
    let pr = describe_in(p, locale);
    let overview = prose::section_html(&pr.overview);
    let jobs: Vec<JobSection> = pr
        .sections
        .iter()
        .map(|sec| JobSection {
            anchor: anchor(&sec.title),
            title: sec.title.clone(),
            html: prose::section_html(&sec.blocks),
        })
        .collect();
    let toc: Vec<TocEntry> = pr
        .sections
        .iter()
        .zip(&p.jobs)
        .map(|(sec, j)| TocEntry {
            title: sec.title.clone(),
            summary: job_summary(j),
            anchor: anchor(&sec.title),
        })
        .collect();
    Runbook {
        overview,
        toc,
        jobs,
        skipped: String::new(),
    }
}

/// JSON form of [`runbook`] for the FFI / UI boundary (default locale).
#[must_use]
pub fn runbook_json(p: &Pipeline) -> String {
    runbook_json_in(p, DEFAULT_LOCALE)
}

/// JSON form of [`runbook_in`] for the FFI / UI boundary, in `locale`.
#[must_use]
pub fn runbook_json_in(p: &Pipeline, locale: &str) -> String {
    serde_json::to_string(&runbook_in(p, locale)).unwrap_or_else(|_| "{}".to_string())
}

/// A one-line summary of a job for the TOC: stage, dep count, step count.
fn job_summary(j: &Job) -> String {
    let mut parts = Vec::new();
    if let Some(st) = &j.stage {
        parts.push(st.clone());
    }
    if !j.needs.is_empty() {
        parts.push(format!("needs {}", j.needs.len()));
    }
    parts.push(format!(
        "{} step{}",
        j.steps.len(),
        if j.steps.len() == 1 { "" } else { "s" }
    ));
    parts.join(" · ")
}

/// A URL-safe anchor from a job name.
fn anchor(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

const HTML_STYLE: &str = r"<style>
.runbook{max-width:760px;margin:0 auto;padding:24px;
  font-family:-apple-system,Segoe UI,Roboto,sans-serif;color:#1b1f24;line-height:1.6}
.runbook h1{font-size:24px;margin:0 0 12px}
.runbook h3{font-size:15px;margin:20px 0 4px;color:#111827}
.runbook p{margin:4px 0}
.runbook code{background:#f6f8fa;border-radius:4px;padding:1px 5px;font-size:12.5px;
  font-family:ui-monospace,Menlo,monospace}
.runbook strong{color:#0f172a}
.runbook em{color:#0e7490;font-style:normal}
.steps{margin:4px 0;padding-left:22px}
.steps li{margin:2px 0}
.job{border-top:1px solid #eef1f5;padding-top:4px}
</style>";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Job, Param, Pipeline, Step};
    use seesaw_core::graph::GhostId;

    fn job(
        name: &str,
        stage: Option<&str>,
        needs: &[&str],
        cmds: &[&str],
        params: &[(&str, &str)],
    ) -> Job {
        Job {
            id: GhostId::from_baseline(name),
            name: name.into(),
            stage: stage.map(Into::into),
            needs: needs.iter().map(|s| (*s).into()).collect(),
            condition: None,
            when: None,
            services: vec![],
            params: params
                .iter()
                .map(|(k, v)| Param {
                    id: GhostId::from_baseline(k),
                    key: (*k).into(),
                    value: (*v).into(),
                })
                .collect(),
            steps: cmds
                .iter()
                .map(|c| Step {
                    id: GhostId::from_baseline(c),
                    label: (*c).into(),
                })
                .collect(),
            byte_start: None,
        }
    }

    fn sample() -> Pipeline {
        Pipeline {
            name: Some("CI".into()),
            stages: vec!["lint".into(), "test".into()],
            jobs: vec![
                job(
                    "fmt",
                    Some("lint"),
                    &[],
                    &["cargo fmt --check"],
                    &[("image", "rust:1.75")],
                ),
                job(
                    "test",
                    Some("test"),
                    &["fmt"],
                    &["cargo test --workspace", "cargo doc"],
                    &[("artifacts", "target/test-results/")],
                ),
            ],
            triggers: vec!["push".into()],
        }
    }

    #[test]
    fn markdown_reads_as_prose_with_command_insets() {
        let m = markdown(&sample());
        assert!(m.starts_with("# CI"));
        // overview is a full sentence
        assert!(m.contains("pipeline defines 2 jobs across the stages"));
        // single-command job → inline command in a sentence
        assert!(m.contains("It runs `cargo fmt --check`."));
        // image clause + stage clause
        assert!(m.contains("runs in the _lint_ stage, inside the `rust:1.75` container"));
        // multi-command job → "in order:" + numbered inset
        assert!(m.contains("It runs the following commands, in order:"));
        assert!(m.contains("1. `cargo test --workspace`"));
        // dependency phrased grammatically
        assert!(m.contains("after **fmt** completes"));
        // artifacts as prose
        assert!(m.contains("publishes its artifacts under `target/test-results/`"));
    }

    #[test]
    fn html_is_prose_with_code_and_sections() {
        let h = html(&sample());
        assert!(h.contains("<h1>CI</h1>"));
        assert!(h.contains("<h3>test</h3>"));
        assert!(h.contains("<code>cargo fmt --check</code>"));
        assert!(h.contains("<strong>fmt</strong>"));
        assert!(h.contains("<em>lint</em>"));
        assert!(h.contains("<ol class=\"steps\">"));
    }

    #[test]
    fn rtf_is_wellformed_and_escaped() {
        let mut p = sample();
        p.jobs[0].steps[0].label = "echo {a} \\ b".into();
        let doc = rtf(&p);
        assert!(doc.starts_with("{\\rtf1") && doc.ends_with('}'));
        assert!(doc.contains("\\{a\\}") && doc.contains("\\\\"));
        assert_eq!(export(&p, "doc").unwrap().1, "rtf");
        assert!(export(&p, "pdf").is_none());
    }

    #[test]
    fn sparse_job_is_honest() {
        let p = Pipeline {
            name: None,
            stages: vec![],
            jobs: vec![job("deploy", Some("deploy"), &[], &[], &[])],
            triggers: vec![],
        };
        let m = markdown(&p);
        assert!(m.contains("This pipeline defines 1 job"));
        assert!(m.contains("steps are not captured in the IR"));
    }

    #[test]
    fn runbook_json_has_overview_and_sections() {
        let rb = runbook(&sample());
        assert!(rb.overview.contains("<p>"));
        assert_eq!(rb.jobs.len(), 2);
        assert!(rb.jobs[0].html.contains("<code>cargo fmt --check</code>"));
        assert_eq!(rb.toc.len(), 2);
        assert!(rb.toc[1].summary.contains("needs 1"));
    }
}
