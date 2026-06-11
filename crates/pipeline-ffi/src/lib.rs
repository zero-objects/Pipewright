//! C-ABI surface over [`pipeline_forward`] + [`pipeline_render`], for the Qt6
//! UI's `BridgeApi` (and any C consumer). One `extern "C"` entry point per
//! `BridgeApi` call. Strings cross the boundary as an opaque [`PipelineString`]
//! (a heap `CString`) that the caller frees via [`pipeline_string_free`]; the
//! payload is UTF-8 — JSON for the structured calls, raw SVG for `render_svg`.
//!
//! Every entry point is panic-guarded: a panic inside Rust is caught and turned
//! into an `{"error": …}` payload rather than unwinding across the FFI boundary
//! (which would be undefined behaviour).
//!
//! Live entry points are backed by the real forward cascade + renderer,
//! including migrate (re-key + backward cascade) and the recipe entry points
//! (list / describe / apply / compose). Capability analysis is the one backend
//! still missing from the `PoC` cut: `pipeline_capabilities_json` returns a
//! structured "not available in this build" error so the UI degrades
//! gracefully instead of crashing.

use std::ffi::{c_char, c_int, CStr, CString};
use std::panic::catch_unwind;
use std::path::PathBuf;

use pipeline_recipe::registry::{RecipeEntry, Registry, SortKey};
use pipeline_render::lift;

/// Opaque owned UTF-8 string handed to C. Free with [`pipeline_string_free`].
pub struct PipelineString {
    s: CString,
}

/// Borrow a C string as `&str`, treating null / invalid UTF-8 as empty.
///
/// # Safety
/// `p` must be null or a valid NUL-terminated C string that outlives the call.
unsafe fn borrow<'a>(p: *const c_char) -> &'a str {
    if p.is_null() {
        return "";
    }
    CStr::from_ptr(p).to_str().unwrap_or("")
}

/// Box a Rust `String` into a `PipelineString` for return to C.
fn ret(s: String) -> *mut PipelineString {
    let s = CString::new(s).unwrap_or_else(|_| CString::new("").unwrap());
    Box::into_raw(Box::new(PipelineString { s }))
}

/// A JSON `{"error": "<msg>"}` payload.
fn err_json(msg: &str) -> String {
    serde_json::json!({ "error": msg }).to_string()
}

/// Run `f`, converting any panic into an `{"error": …}` JSON payload.
fn guard(f: impl FnOnce() -> String + std::panic::UnwindSafe) -> *mut PipelineString {
    match catch_unwind(f) {
        Ok(s) => ret(s),
        Err(_) => ret(err_json("internal error (panic) in pipeline-ffi")),
    }
}

/// Pointer to the UTF-8 bytes of a `PipelineString` (NUL-terminated).
///
/// # Safety
/// `p` must be a pointer returned by this library and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn pipeline_string_data(p: *const PipelineString) -> *const c_char {
    if p.is_null() {
        return std::ptr::null();
    }
    (*p).s.as_ptr()
}

/// Free a `PipelineString` returned by this library.
///
/// # Safety
/// `p` must be a pointer returned by this library and not freed before.
#[no_mangle]
pub unsafe extern "C" fn pipeline_string_free(p: *mut PipelineString) {
    if !p.is_null() {
        drop(Box::from_raw(p));
    }
}

/// The library version string.
#[no_mangle]
pub extern "C" fn pipeline_ffi_version() -> *mut PipelineString {
    ret(env!("CARGO_PKG_VERSION").to_string())
}

/// The platforms the engine supports, as a JSON array of keys
/// (`["argo","aws_codebuild",…]`). The single source of truth for the UI's
/// platform pickers — so they never drift from [`pipeline_forward::PLATFORMS`].
#[no_mangle]
pub extern "C" fn pipeline_platforms() -> *mut PipelineString {
    ret(serde_json::json!(pipeline_forward::PLATFORMS).to_string())
}

/// The platform's conventional pipeline file name (what a save dialog should
/// suggest), as JSON `{"name": "<file>"}`. Unknown platforms get the generic
/// `pipeline.yml`.
///
/// # Safety
/// `kind` must be null or a valid C string.
#[no_mangle]
pub unsafe extern "C" fn pipeline_default_file_name(kind: *const c_char) -> *mut PipelineString {
    let kind = borrow(kind).to_string();
    guard(move || {
        serde_json::json!({ "name": pipeline_forward::default_file_name(&kind) }).to_string()
    })
}

/// Whether `kind`'s pipelines can be run locally in Docker, as JSON
/// `{"runnable": <bool>, "reason": "<why not>"}`. `reason` is empty when
/// runnable. Lets the UI disable the Run button for translate-only platforms
/// instead of offering a dead click.
///
/// # Safety
/// `kind` must be null or a valid C string.
#[no_mangle]
pub unsafe extern "C" fn pipeline_run_support(kind: *const c_char) -> *mut PipelineString {
    let kind = borrow(kind).to_string();
    guard(move || {
        let full = pipeline_forward::run_support(&kind) == pipeline_forward::RunSupport::Full;
        let reason = if full {
            ""
        } else {
            pipeline_forward::RunSupport::reason(&kind)
        };
        serde_json::json!({ "runnable": full, "reason": reason }).to_string()
    })
}

/// Best-effort platform detection from source text. Returns JSON
/// `{"kind": "<platform>"}` (a key from [`pipeline_forward::PLATFORMS`]), or
/// `{"kind": "unknown"}` if nothing matches.
///
/// # Safety
/// `yaml` must be null or a valid C string.
#[no_mangle]
pub unsafe extern "C" fn pipeline_detect_source_kind(yaml: *const c_char) -> *mut PipelineString {
    let src = borrow(yaml).to_string();
    guard(move || {
        let kind = detect_kind(&src);
        let kind = if kind.is_empty() { "unknown" } else { kind };
        serde_json::json!({ "kind": kind }).to_string()
    })
}

/// Detection that prefers the file NAME (`.gitlab-ci.yml`, `Jenkinsfile`, …)
/// over the content — the same logic the CLI uses. `path` may be empty to fall
/// back to pure content detection. Returns `{"kind": "<platform>|unknown"}`.
///
/// # Safety
/// `path` and `yaml` must be null or valid C strings.
#[no_mangle]
pub unsafe extern "C" fn pipeline_detect_with_path(
    path: *const c_char,
    yaml: *const c_char,
) -> *mut PipelineString {
    let (path, src) = (borrow(path).to_string(), borrow(yaml).to_string());
    guard(move || {
        let kind = if path.is_empty() {
            pipeline_forward::detect(&src)
        } else {
            pipeline_forward::detect_with_path(&path, &src)
        }
        .unwrap_or("unknown");
        serde_json::json!({ "kind": kind }).to_string()
    })
}

/// Forward `yaml` for `kind` and return the SVG diagram (raw markup).
///
/// # Safety
/// All pointers must be null or valid C strings.
#[no_mangle]
pub unsafe extern "C" fn pipeline_render_svg(
    yaml: *const c_char,
    kind: *const c_char,
    _trigger: *const c_char,
    _ref: *const c_char,
    _show_scripts: c_int,
    _show_capabilities: c_int,
    _include_root: *const c_char,
) -> *mut PipelineString {
    let (src, kind) = (borrow(yaml).to_string(), borrow(kind).to_string());
    guard(move || match lift_pipeline(&kind, &src) {
        Ok(p) => pipeline_render::render_diagram(&p).svg,
        Err(e) => err_svg(&e),
    })
}

/// Forward `yaml` for `kind` and return `{svg, layout:{width,height,jobs}}`.
///
/// # Safety
/// All pointers must be null or valid C strings.
#[no_mangle]
pub unsafe extern "C" fn pipeline_render_svg_with_layout(
    yaml: *const c_char,
    kind: *const c_char,
    _trigger: *const c_char,
    _ref: *const c_char,
    _show_scripts: c_int,
    _show_capabilities: c_int,
    _include_root: *const c_char,
) -> *mut PipelineString {
    let (src, kind) = (borrow(yaml).to_string(), borrow(kind).to_string());
    guard(move || match lift_pipeline(&kind, &src) {
        Ok(p) => {
            let d = pipeline_render::render_diagram(&p);
            serde_json::to_string(&d).unwrap_or_else(|_| err_json("diagram serialisation failed"))
        }
        Err(e) => err_json(&e),
    })
}

/// Forward `yaml` for `kind` and return the runbook `{overview, toc, jobs,
/// skipped}` JSON.
///
/// # Safety
/// All pointers must be null or valid C strings.
#[no_mangle]
pub unsafe extern "C" fn pipeline_render_html(
    yaml: *const c_char,
    kind: *const c_char,
    _trigger: *const c_char,
    _ref: *const c_char,
    _include_root: *const c_char,
) -> *mut PipelineString {
    let (src, kind) = (borrow(yaml).to_string(), borrow(kind).to_string());
    guard(move || match lift_pipeline(&kind, &src) {
        Ok(p) => pipeline_render::runbook_json(&p),
        Err(e) => err_json(&e),
    })
}

/// Like [`pipeline_render_html`] but in `locale` (e.g. "en" / "de"). An empty
/// locale falls back to the default. The runbook prose is localized.
///
/// # Safety
/// All pointers must be null or valid C strings.
#[no_mangle]
pub unsafe extern "C" fn pipeline_render_html_in(
    yaml: *const c_char,
    kind: *const c_char,
    locale: *const c_char,
    _trigger: *const c_char,
    _ref: *const c_char,
    _include_root: *const c_char,
) -> *mut PipelineString {
    let (src, kind, locale) = (
        borrow(yaml).to_string(),
        borrow(kind).to_string(),
        borrow(locale).to_string(),
    );
    guard(move || match lift_pipeline(&kind, &src) {
        Ok(p) => {
            let loc = if locale.is_empty() {
                pipeline_render::DEFAULT_LOCALE
            } else {
                &locale
            };
            pipeline_render::runbook_json_in(&p, loc)
        }
        Err(e) => err_json(&e),
    })
}

/// Forward `yaml` for `kind` and return `{pipeline:{name,jobs:[…]}}`.
///
/// # Safety
/// All pointers must be null or valid C strings.
#[no_mangle]
pub unsafe extern "C" fn pipeline_inspect_json(
    yaml: *const c_char,
    kind: *const c_char,
    _trigger: *const c_char,
    _ref: *const c_char,
    _include_root: *const c_char,
) -> *mut PipelineString {
    let (src, kind) = (borrow(yaml).to_string(), borrow(kind).to_string());
    guard(move || match lift_pipeline(&kind, &src) {
        Ok(p) => pipeline_render::inspect_json(&p),
        Err(e) => err_json(&e),
    })
}

/// Apply a value edit and return `{"ok":true,"yaml":"<new source>"}` (or
/// `{"error":…}`). `hub` is the 64-char `GhostId` hex from a diagram line's
/// `data-hub`; the field is changed *on the Hub-IR* and the source is
/// regenerated through the backward TGG cascade (the IR is canonical, so the
/// output normalises — formatting/key-order, not a byte patch).
///
/// # Safety
/// All pointers must be null or valid C strings.
#[no_mangle]
pub unsafe extern "C" fn pipeline_edit_field(
    yaml: *const c_char,
    kind: *const c_char,
    hub: *const c_char,
    new_value: *const c_char,
) -> *mut PipelineString {
    let (src, kind, hub, val) = (
        borrow(yaml).to_string(),
        borrow(kind).to_string(),
        borrow(hub).to_string(),
        borrow(new_value).to_string(),
    );
    guard(move || {
        if kind.is_empty() {
            return err_json("no source kind selected");
        }
        match pipeline_forward::edit(&kind, &src, &hub, &val) {
            Ok(new_yaml) => serde_json::json!({ "ok": true, "yaml": new_yaml }).to_string(),
            Err(e) => err_json(&e.to_string()),
        }
    })
}

/// Duplicate the construct at `hub` (a job or step): clone it in place via the
/// Hub-IR and re-emit. Returns `{"ok":true,"yaml":…}` | `{"error":…}`.
///
/// # Safety
/// All pointers must be null or valid C strings.
#[no_mangle]
pub unsafe extern "C" fn pipeline_duplicate(
    yaml: *const c_char,
    kind: *const c_char,
    hub: *const c_char,
) -> *mut PipelineString {
    structural(
        borrow(yaml),
        borrow(kind),
        borrow(hub),
        pipeline_forward::duplicate,
    )
}

/// Delete the construct at `hub` (a job or step) from the Hub-IR and re-emit.
/// Returns `{"ok":true,"yaml":…}` | `{"error":…}`.
///
/// # Safety
/// All pointers must be null or valid C strings.
#[no_mangle]
pub unsafe extern "C" fn pipeline_delete(
    yaml: *const c_char,
    kind: *const c_char,
    hub: *const c_char,
) -> *mut PipelineString {
    structural(
        borrow(yaml),
        borrow(kind),
        borrow(hub),
        pipeline_forward::delete,
    )
}

/// Shared body for the structural ops (duplicate / delete): forward + mutate +
/// re-emit, wrapped as `{"ok":true,"yaml":…}` | `{"error":…}`.
fn structural(
    yaml: &str,
    kind: &str,
    hub: &str,
    op: fn(&str, &str, &str) -> Result<String, pipeline_forward::ForwardError>,
) -> *mut PipelineString {
    let (src, kind, hub) = (yaml.to_string(), kind.to_string(), hub.to_string());
    guard(move || {
        if kind.is_empty() {
            return err_json("no source kind selected");
        }
        match op(&kind, &src, &hub) {
            Ok(new_yaml) => serde_json::json!({ "ok": true, "yaml": new_yaml }).to_string(),
            Err(e) => err_json(&e.to_string()),
        }
    })
}

/// Export the human-readable runbook in `format` ("md" | "html" | "doc"). On
/// success returns `{"ok":true,"content":"…","ext":"md|html|rtf"}`; otherwise
/// `{"error":…}`. The IR is canonical, so every format is a clean projection.
///
/// # Safety
/// All pointers must be null or valid C strings.
#[no_mangle]
pub unsafe extern "C" fn pipeline_export_runbook(
    yaml: *const c_char,
    kind: *const c_char,
    format: *const c_char,
) -> *mut PipelineString {
    let (src, kind, fmt) = (
        borrow(yaml).to_string(),
        borrow(kind).to_string(),
        borrow(format).to_string(),
    );
    guard(move || match lift_pipeline(&kind, &src) {
        Ok(p) => match pipeline_render::export(&p, &fmt) {
            Some((content, ext)) => {
                serde_json::json!({ "ok": true, "content": content, "ext": ext }).to_string()
            }
            None => err_json(&format!("unknown export format: {fmt}")),
        },
        Err(e) => err_json(&e),
    })
}

/// Like [`pipeline_export_runbook`] but in `locale` (e.g. "en" / "de"). An empty
/// locale falls back to the default.
///
/// # Safety
/// All pointers must be null or valid C strings.
#[no_mangle]
pub unsafe extern "C" fn pipeline_export_runbook_in(
    yaml: *const c_char,
    kind: *const c_char,
    format: *const c_char,
    locale: *const c_char,
) -> *mut PipelineString {
    let (src, kind, fmt, locale) = (
        borrow(yaml).to_string(),
        borrow(kind).to_string(),
        borrow(format).to_string(),
        borrow(locale).to_string(),
    );
    guard(move || match lift_pipeline(&kind, &src) {
        Ok(p) => {
            let loc = if locale.is_empty() {
                pipeline_render::DEFAULT_LOCALE
            } else {
                &locale
            };
            match pipeline_render::export_in(&p, &fmt, loc) {
                Some((content, ext)) => {
                    serde_json::json!({ "ok": true, "content": content, "ext": ext }).to_string()
                }
                None => err_json(&format!("unknown export format: {fmt}")),
            }
        }
        Err(e) => err_json(&e),
    })
}

/// Capability profile of a pipeline, derived from the Hub-IR via the shared
/// [`pipeline_render::capabilities_json`]. Returns
/// `{overall, summary, jobs, steps, features}` | `{error}`.
///
/// # Safety
/// All pointers must be null or valid C strings.
#[no_mangle]
pub unsafe extern "C" fn pipeline_capabilities_json(
    yaml: *const c_char,
    kind: *const c_char,
    _include_root: *const c_char,
) -> *mut PipelineString {
    let (src, kind) = (borrow(yaml).to_string(), borrow(kind).to_string());
    guard(move || {
        if kind.is_empty() {
            return err_json("no source kind selected");
        }
        match pipeline_forward::forward(&kind, &src) {
            Ok(g) => pipeline_render::capabilities_json(&g),
            Err(e) => err_json(&e.to_string()),
        }
    })
}

/// Migrate `yaml` from platform `kind` to `target`: forward to the Hub-IR,
/// re-key to the target vocabulary, re-emit. Returns `{"ok":true,"yaml":…}` |
/// `{"error":…}`. Fidelity is best between structurally compatible platforms.
///
/// # Safety
/// All pointers must be null or valid C strings.
#[no_mangle]
pub unsafe extern "C" fn pipeline_migrate(
    yaml: *const c_char,
    kind: *const c_char,
    target: *const c_char,
    _include_root: *const c_char,
) -> *mut PipelineString {
    let (src, kind, target) = (
        borrow(yaml).to_string(),
        borrow(kind).to_string(),
        borrow(target).to_string(),
    );
    guard(move || {
        if kind.is_empty() || target.is_empty() {
            return err_json("source kind and target are both required");
        }
        match pipeline_forward::migrate_with_report(&kind, &src, &target) {
            Ok((out, report)) if !out.trim().is_empty() && out.trim() != "{}" => {
                let items: Vec<_> = report
                    .iter()
                    .map(|f| serde_json::json!({ "severity": f.severity, "feature": f.feature, "note": f.note }))
                    .collect();
                serde_json::json!({ "ok": true, "yaml": out, "report": { "items": items } })
                    .to_string()
            }
            Ok(_) => err_json(&format!(
                "'{kind}' → '{target}' produced nothing (incompatible platform structures)"
            )),
            Err(e) => err_json(&e.to_string()),
        }
    })
}

/// Compose the recipe files at `paths` into a pipeline rendered to `target`.
/// Returns `{"ok":true,"yaml":…}` | `{"error":…}`.
///
/// # Safety
/// `paths` must point to `count` valid C strings; `target` null or valid.
#[no_mangle]
pub unsafe extern "C" fn pipeline_compose_recipes(
    paths: *const *const c_char,
    count: usize,
    target: *const c_char,
) -> *mut PipelineString {
    let target = borrow(target).to_string();
    let files: Vec<String> = if paths.is_null() {
        vec![]
    } else {
        (0..count)
            .map(|i| borrow(*paths.add(i)).to_string())
            .collect()
    };
    guard(move || {
        if target.is_empty() {
            return err_json("no target platform selected");
        }
        let mut docs = Vec::new();
        for path in &files {
            match std::fs::read_to_string(path) {
                Ok(c) => docs.push(c),
                Err(e) => return err_json(&format!("cannot read {path}: {e}")),
            }
        }
        match pipeline_recipe::compose_documents(&docs, &target) {
            Ok(out) => serde_json::json!({ "ok": true, "yaml": out }).to_string(),
            Err(e) => err_json(&e.to_string()),
        }
    })
}

/// Build a recipe registry: the embedded standard library plus, if
/// `config_path` is non-empty, the user sources it declares (git sources cloned
/// into `cache_dir`, or a default cache if empty). Returns the registry and any
/// non-fatal warnings (a bad source never aborts the others).
fn build_registry(config_path: &str, cache_dir: &str) -> (Registry, Vec<String>) {
    let mut reg = Registry::with_standard();
    let mut warns = Vec::new();
    if config_path.is_empty() {
        return (reg, warns);
    }
    let text = match std::fs::read_to_string(config_path) {
        Ok(t) => t,
        Err(e) => {
            warns.push(format!("config: cannot read {config_path}: {e}"));
            return (reg, warns);
        }
    };
    match pipeline_recipe::config::load_config(&text) {
        Ok(cfg) => {
            let cache = if cache_dir.is_empty() {
                std::env::temp_dir().join("pipewright-recipe-cache")
            } else {
                PathBuf::from(cache_dir)
            };
            for (label, e) in reg.load_config(&cfg, &cache) {
                warns.push(format!("{label}: {e}"));
            }
        }
        Err(e) => warns.push(format!("config: {e}")),
    }
    (reg, warns)
}

/// The JSON shape of a recipe for the UI's browse list.
fn recipe_json(entry: &RecipeEntry) -> serde_json::Value {
    let r = &entry.recipe;
    let ports = |ps: &[pipeline_recipe::Port]| {
        ps.iter()
            .map(|p| serde_json::json!({ "name": p.name, "kind": p.kind }))
            .collect::<Vec<_>>()
    };
    serde_json::json!({
        "id": r.recipe_id,
        "version": r.recipe_version,
        "description": r.description,
        "doc": r.doc,
        "tags": r.tags,
        "requirements": r.platform_requirements,
        "inputs": ports(&r.input_ports),
        "outputs": ports(&r.output_ports),
        "jobs": r.jobs.keys().collect::<Vec<_>>(),
        "source": entry.source.label(),
    })
}

/// List recipes for the browser: standard library + configured user sources,
/// filtered by `query` (id/description/tags substring) and ordered by `sort`
/// ("name" | "tag" | "source"). Returns
/// `{"ok":true,"recipes":[…],"warnings":[…]}`.
///
/// # Safety
/// All pointers must be null or valid C strings.
#[no_mangle]
pub unsafe extern "C" fn pipeline_list_recipes(
    query: *const c_char,
    sort: *const c_char,
    config_path: *const c_char,
    cache_dir: *const c_char,
) -> *mut PipelineString {
    let (query, sort, config_path, cache_dir) = (
        borrow(query).to_string(),
        borrow(sort).to_string(),
        borrow(config_path).to_string(),
        borrow(cache_dir).to_string(),
    );
    guard(move || {
        let (reg, warns) = build_registry(&config_path, &cache_dir);
        let key = SortKey::from_str_or_name(&sort);
        let recipes: Vec<serde_json::Value> = reg
            .browse(&query, key)
            .iter()
            .map(|e| recipe_json(e))
            .collect();
        serde_json::json!({ "ok": true, "recipes": recipes, "warnings": warns }).to_string()
    })
}

/// A generated, localized structural description (Markdown) of a recipe, via the
/// prose doc mechanism. `locale` is e.g. "en" / "de". Returns
/// `{"ok":true,"markdown":…}` | `{"error":…}`.
///
/// # Safety
/// All pointers must be null or valid C strings.
#[no_mangle]
pub unsafe extern "C" fn pipeline_describe_recipe(
    recipe_id: *const c_char,
    locale: *const c_char,
    config_path: *const c_char,
    cache_dir: *const c_char,
) -> *mut PipelineString {
    let (recipe_id, locale, config_path, cache_dir) = (
        borrow(recipe_id).to_string(),
        borrow(locale).to_string(),
        borrow(config_path).to_string(),
        borrow(cache_dir).to_string(),
    );
    guard(move || {
        let (reg, _warns) = build_registry(&config_path, &cache_dir);
        let Some(entry) = reg.get(&recipe_id) else {
            return err_json(&format!("no recipe with id '{recipe_id}'"));
        };
        let loc = if locale.is_empty() {
            pipeline_render::DEFAULT_LOCALE
        } else {
            &locale
        };
        match pipeline_recipe::describe_recipe(&entry.recipe, loc) {
            Ok(md) => serde_json::json!({ "ok": true, "markdown": md }).to_string(),
            Err(e) => err_json(&e.to_string()),
        }
    })
}

/// Apply the recipe `recipe_id` to the current pipeline `yaml` of platform
/// `kind`: the recipe's jobs are merged in and the result re-emitted in the same
/// platform — the graph-edit "apply recipe" operation. An empty `yaml` starts a
/// fresh pipeline. Returns `{"ok":true,"yaml":…}` | `{"error":…}`.
///
/// # Safety
/// All pointers must be null or valid C strings.
#[no_mangle]
pub unsafe extern "C" fn pipeline_apply_recipe(
    yaml: *const c_char,
    kind: *const c_char,
    recipe_id: *const c_char,
    config_path: *const c_char,
    cache_dir: *const c_char,
) -> *mut PipelineString {
    let (src, kind, recipe_id, config_path, cache_dir) = (
        borrow(yaml).to_string(),
        borrow(kind).to_string(),
        borrow(recipe_id).to_string(),
        borrow(config_path).to_string(),
        borrow(cache_dir).to_string(),
    );
    guard(move || {
        if kind.is_empty() {
            return err_json("no source kind selected");
        }
        let (reg, _warns) = build_registry(&config_path, &cache_dir);
        let Some(entry) = reg.get(&recipe_id) else {
            return err_json(&format!("no recipe with id '{recipe_id}'"));
        };
        match pipeline_recipe::apply_to_source(&src, &kind, &entry.recipe) {
            Ok(out) => serde_json::json!({ "ok": true, "yaml": out }).to_string(),
            Err(e) => err_json(&e.to_string()),
        }
    })
}

/// Forward + lift a pipeline, mapping every failure to a human-readable string.
fn lift_pipeline(kind: &str, src: &str) -> Result<pipeline_render::Pipeline, String> {
    if kind.is_empty() {
        return Err("no source kind selected".to_string());
    }
    let g = pipeline_forward::forward(kind, src).map_err(|e| e.to_string())?;
    lift(&g).ok_or_else(|| "the IR holds no pipeline to render".to_string())
}

/// An SVG carrying an error message — so the diagram pane shows the reason
/// rather than going blank.
fn err_svg(msg: &str) -> String {
    let esc = msg
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="420" height="60"><text x="12" y="34" font-family="sans-serif" font-size="13" fill="#b91c1c">{esc}</text></svg>"##
    )
}

/// Heuristic platform detection — shared with the CLI via
/// [`pipeline_forward::detect`]. Returns `""` when nothing matches.
fn detect_kind(src: &str) -> &'static str {
    pipeline_forward::detect(src).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip a C call: build args, invoke, read the string, free it.
    unsafe fn call_layout(yaml: &str, kind: &str) -> String {
        let y = CString::new(yaml).unwrap();
        let k = CString::new(kind).unwrap();
        let empty = CString::new("").unwrap();
        let p = pipeline_render_svg_with_layout(
            y.as_ptr(),
            k.as_ptr(),
            empty.as_ptr(),
            empty.as_ptr(),
            0,
            0,
            empty.as_ptr(),
        );
        let out = CStr::from_ptr(pipeline_string_data(p))
            .to_str()
            .unwrap()
            .to_string();
        pipeline_string_free(p);
        out
    }

    const GITLAB: &str = "build:\n  script:\n    - cargo build\ntest:\n  needs:\n    - build\n  script:\n    - cargo test\n";

    #[test]
    fn version_round_trips() {
        unsafe {
            let p = pipeline_ffi_version();
            let v = CStr::from_ptr(pipeline_string_data(p))
                .to_str()
                .unwrap()
                .to_string();
            pipeline_string_free(p);
            assert!(!v.is_empty());
        }
    }

    #[test]
    fn svg_with_layout_has_svg_and_jobs() {
        let out = unsafe { call_layout(GITLAB, "gitlab") };
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(v["svg"].as_str().unwrap().contains("<svg"));
        assert_eq!(v["layout"]["jobs"].as_array().unwrap().len(), 2);
        // The hub GhostId anchor rides along for the reverse editor.
        assert!(v["layout"]["jobs"][0]["hub"].as_str().unwrap().len() == 64);
    }

    #[test]
    fn human_and_inspect_round_trip() {
        unsafe {
            let y = CString::new(GITLAB).unwrap();
            let k = CString::new("gitlab").unwrap();
            let e = CString::new("").unwrap();
            let h =
                pipeline_render_html(y.as_ptr(), k.as_ptr(), e.as_ptr(), e.as_ptr(), e.as_ptr());
            let hs = CStr::from_ptr(pipeline_string_data(h))
                .to_str()
                .unwrap()
                .to_string();
            pipeline_string_free(h);
            let hv: serde_json::Value = serde_json::from_str(&hs).unwrap();
            assert_eq!(hv["jobs"].as_array().unwrap().len(), 2);
            assert!(hv["overview"].as_str().unwrap().contains("job"));

            let i =
                pipeline_inspect_json(y.as_ptr(), k.as_ptr(), e.as_ptr(), e.as_ptr(), e.as_ptr());
            let is = CStr::from_ptr(pipeline_string_data(i))
                .to_str()
                .unwrap()
                .to_string();
            pipeline_string_free(i);
            let iv: serde_json::Value = serde_json::from_str(&is).unwrap();
            assert_eq!(iv["pipeline"]["jobs"].as_array().unwrap().len(), 2);
        }
    }

    #[test]
    fn unknown_kind_returns_error_json_not_panic() {
        let out = unsafe { call_layout("x: 1", "nope") };
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(v["error"].is_string());
    }

    #[test]
    fn edit_field_round_trips_through_ffi() {
        unsafe {
            // Find the image node's hex via the render model isn't available here,
            // so drive it through the layout descriptor: the first job box's hub
            // is a job node (no prov-editable scalar) — instead edit a known value
            // by forwarding in Rust. We assert the FFI wrapper shape via a bad id
            // (graceful) and a real id (applied) using drone's image node.
            let src = "kind: pipeline\nname: ci\nsteps:\n  - name: build\n    image: rust:1.75\n    commands:\n      - cargo build\n";
            let g = pipeline_forward::forward("drone", src).unwrap();
            let hub = g
                .iter_nodes()
                .find(|n| n.type_id == "hub:image")
                .unwrap()
                .id
                .hex();
            let (y, k, h, v) = (
                CString::new(src).unwrap(),
                CString::new("drone").unwrap(),
                CString::new(hub).unwrap(),
                CString::new("rust:1.80").unwrap(),
            );
            let p = pipeline_edit_field(y.as_ptr(), k.as_ptr(), h.as_ptr(), v.as_ptr());
            let out = CStr::from_ptr(pipeline_string_data(p))
                .to_str()
                .unwrap()
                .to_string();
            pipeline_string_free(p);
            let val: serde_json::Value = serde_json::from_str(&out).unwrap();
            assert_eq!(val["ok"], true);
            assert!(val["yaml"].as_str().unwrap().contains("image: rust:1.80"));
        }
    }

    /// Invoke a C entry point with C-string args, read+free the result.
    unsafe fn call(p: *mut PipelineString) -> serde_json::Value {
        let s = CStr::from_ptr(pipeline_string_data(p))
            .to_str()
            .unwrap()
            .to_string();
        pipeline_string_free(p);
        serde_json::from_str(&s).unwrap()
    }

    #[test]
    fn platforms_lists_all_supported() {
        unsafe {
            let v = call(pipeline_platforms());
            let arr = v.as_array().expect("array");
            assert_eq!(arr.len(), pipeline_forward::PLATFORMS.len());
            assert!(arr.iter().any(|p| p == "gitlab") && arr.iter().any(|p| p == "drone"));
        }
    }

    #[test]
    fn detect_with_path_prefers_filename() {
        unsafe {
            // github-looking content, but named .gitlab-ci.yml → gitlab.
            let content = "on:\n  script: [x]\njobs:\n  script: [y]\n";
            let (path, yaml) = (
                CString::new("/r/.gitlab-ci.yml").unwrap(),
                CString::new(content).unwrap(),
            );
            let v = call(pipeline_detect_with_path(path.as_ptr(), yaml.as_ptr()));
            assert_eq!(v["kind"], "gitlab");
            // empty path → content detection.
            let (empty, yaml) = (CString::new("").unwrap(), CString::new(content).unwrap());
            let v = call(pipeline_detect_with_path(empty.as_ptr(), yaml.as_ptr()));
            assert_eq!(v["kind"], "github");
        }
    }

    #[test]
    fn run_support_reports_runnable_with_reason() {
        unsafe {
            let k = CString::new("gitlab").unwrap();
            let v = call(pipeline_run_support(k.as_ptr()));
            assert_eq!(v["runnable"], true);
            assert_eq!(v["reason"], "");
            // translate-only platform: not runnable, with a non-empty reason.
            let k = CString::new("argo").unwrap();
            let v = call(pipeline_run_support(k.as_ptr()));
            assert_eq!(v["runnable"], false);
            assert!(!v["reason"].as_str().unwrap().is_empty());
        }
    }

    #[test]
    fn default_file_name_per_platform() {
        unsafe {
            let k = CString::new("gitlab").unwrap();
            assert_eq!(
                call(pipeline_default_file_name(k.as_ptr()))["name"],
                ".gitlab-ci.yml"
            );
            let k = CString::new("jenkins").unwrap();
            assert_eq!(
                call(pipeline_default_file_name(k.as_ptr()))["name"],
                "Jenkinsfile"
            );
            // Unknown platform falls back to the generic name.
            let k = CString::new("nope").unwrap();
            assert_eq!(
                call(pipeline_default_file_name(k.as_ptr()))["name"],
                "pipeline.yml"
            );
        }
    }

    #[test]
    fn list_recipes_returns_standard_library() {
        unsafe {
            let (q, sort, cfg, cache) = (
                CString::new("rust").unwrap(),
                CString::new("name").unwrap(),
                CString::new("").unwrap(),
                CString::new("").unwrap(),
            );
            let v = call(pipeline_list_recipes(
                q.as_ptr(),
                sort.as_ptr(),
                cfg.as_ptr(),
                cache.as_ptr(),
            ));
            assert_eq!(v["ok"], true);
            let recipes = v["recipes"].as_array().unwrap();
            assert!(
                recipes.iter().any(|r| r["id"] == "rust-ci"),
                "rust-ci in results: {v}"
            );
            assert_eq!(
                recipes.iter().find(|r| r["id"] == "rust-ci").unwrap()["source"],
                "standard"
            );
        }
    }

    #[test]
    fn describe_recipe_is_localized() {
        unsafe {
            let (id, en, de, cfg, cache) = (
                CString::new("rust-ci").unwrap(),
                CString::new("en").unwrap(),
                CString::new("de").unwrap(),
                CString::new("").unwrap(),
                CString::new("").unwrap(),
            );
            let v_en = call(pipeline_describe_recipe(
                id.as_ptr(),
                en.as_ptr(),
                cfg.as_ptr(),
                cache.as_ptr(),
            ));
            let v_de = call(pipeline_describe_recipe(
                id.as_ptr(),
                de.as_ptr(),
                cfg.as_ptr(),
                cache.as_ptr(),
            ));
            assert_eq!(v_en["ok"], true);
            assert_eq!(v_de["ok"], true);
            assert_ne!(
                v_en["markdown"], v_de["markdown"],
                "locale changes the prose"
            );
        }
    }

    #[test]
    fn apply_recipe_merges_into_current_pipeline() {
        unsafe {
            let (src, kind, id, cfg, cache) = (
                CString::new("build:\n  script:\n    - make\n").unwrap(),
                CString::new("gitlab").unwrap(),
                CString::new("rust-ci").unwrap(),
                CString::new("").unwrap(),
                CString::new("").unwrap(),
            );
            let v = call(pipeline_apply_recipe(
                src.as_ptr(),
                kind.as_ptr(),
                id.as_ptr(),
                cfg.as_ptr(),
                cache.as_ptr(),
            ));
            assert_eq!(v["ok"], true, "{v}");
            let yaml = v["yaml"].as_str().unwrap();
            assert!(
                yaml.contains("build") && yaml.contains("rust-ci-lint"),
                "merged: {yaml}"
            );
        }
    }

    #[test]
    fn apply_unknown_recipe_errors_gracefully() {
        unsafe {
            let (src, kind, id, cfg, cache) = (
                CString::new("build:\n  script:\n    - make\n").unwrap(),
                CString::new("gitlab").unwrap(),
                CString::new("does-not-exist").unwrap(),
                CString::new("").unwrap(),
                CString::new("").unwrap(),
            );
            let v = call(pipeline_apply_recipe(
                src.as_ptr(),
                kind.as_ptr(),
                id.as_ptr(),
                cfg.as_ptr(),
                cache.as_ptr(),
            ));
            assert!(v["error"].is_string());
        }
    }

    #[test]
    fn export_runbook_formats() {
        unsafe {
            let y = CString::new(GITLAB).unwrap();
            let k = CString::new("gitlab").unwrap();
            for (fmt, ext, marker) in [
                ("md", "md", "pipeline defines"),
                ("html", "html", "<h1>"),
                ("doc", "rtf", "\\rtf1"),
            ] {
                let f = CString::new(fmt).unwrap();
                let p = pipeline_export_runbook(y.as_ptr(), k.as_ptr(), f.as_ptr());
                let out = CStr::from_ptr(pipeline_string_data(p))
                    .to_str()
                    .unwrap()
                    .to_string();
                pipeline_string_free(p);
                let v: serde_json::Value = serde_json::from_str(&out).unwrap();
                assert_eq!(v["ok"], true, "{fmt}: {out}");
                assert_eq!(v["ext"], ext);
                assert!(
                    v["content"].as_str().unwrap().contains(marker),
                    "{fmt} missing {marker}"
                );
            }
        }
    }

    #[test]
    fn duplicate_and_delete_round_trip() {
        unsafe {
            let src = "build:\n  script:\n    - cargo build\ntest:\n  needs:\n    - build\n  script:\n    - cargo test\n";
            let g = pipeline_forward::forward("gitlab", src).unwrap();
            let job = g
                .iter_nodes()
                .find(|n| n.type_id == "hub:job")
                .unwrap()
                .id
                .hex();
            let (y, k, h) = (
                CString::new(src).unwrap(),
                CString::new("gitlab").unwrap(),
                CString::new(job).unwrap(),
            );
            let d = pipeline_delete(y.as_ptr(), k.as_ptr(), h.as_ptr());
            let out = CStr::from_ptr(pipeline_string_data(d))
                .to_str()
                .unwrap()
                .to_string();
            pipeline_string_free(d);
            let v: serde_json::Value = serde_json::from_str(&out).unwrap();
            assert_eq!(v["ok"], true);
            assert!(v["yaml"].is_string());
        }
    }

    #[test]
    fn migrate_round_trips() {
        unsafe {
            let src = "build:\n  script:\n    - cargo build\n";
            let (y, k, t, e) = (
                CString::new(src).unwrap(),
                CString::new("gitlab").unwrap(),
                CString::new("gitlab").unwrap(),
                CString::new("").unwrap(),
            );
            let m = pipeline_migrate(y.as_ptr(), k.as_ptr(), t.as_ptr(), e.as_ptr());
            let ms = CStr::from_ptr(pipeline_string_data(m))
                .to_str()
                .unwrap()
                .to_string();
            pipeline_string_free(m);
            let v: serde_json::Value = serde_json::from_str(&ms).unwrap();
            assert_eq!(v["ok"], true);
            assert!(v["yaml"].as_str().unwrap().contains("cargo build"));
            // The report key is always present (empty items for a clean migration).
            assert!(v["report"]["items"].is_array());
        }
    }

    #[test]
    fn migrate_reports_dropped_capabilities() {
        unsafe {
            // gitlab → drone drops cache + services → the report is non-empty,
            // each item carrying severity + feature + note (what the UI renders).
            let src = "build:\n  image: rust:1.75\n  cache:\n    paths: [target]\n  services:\n    - postgres:16\n  script:\n    - cargo test\n";
            let (y, k, t, e) = (
                CString::new(src).unwrap(),
                CString::new("gitlab").unwrap(),
                CString::new("drone").unwrap(),
                CString::new("").unwrap(),
            );
            let v = call(pipeline_migrate(
                y.as_ptr(),
                k.as_ptr(),
                t.as_ptr(),
                e.as_ptr(),
            ));
            let items = v["report"]["items"].as_array().expect("items array");
            assert!(!items.is_empty(), "lossy migration reports friction: {v}");
            assert!(items.iter().all(|i| i["severity"].is_string()
                && i["feature"].is_string()
                && i["note"].is_string()));
        }
    }

    #[test]
    fn capabilities_profiles_the_pipeline() {
        unsafe {
            // A plain job+image only → Possible (image is universal).
            let plain =
                CString::new("build:\n  image: rust:1.75\n  script:\n    - cargo build\n").unwrap();
            let k = CString::new("gitlab").unwrap();
            let e = CString::new("").unwrap();
            let v = call(pipeline_capabilities_json(
                plain.as_ptr(),
                k.as_ptr(),
                e.as_ptr(),
            ));
            assert_eq!(v["overall"], "Possible", "{v}");
            assert!(v["jobs"].as_u64().unwrap() >= 1);
            assert!(v["features"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["key"] == "image"));

            // Add a service + cache → non-universal features → caveats.
            let rich = CString::new(
                "build:\n  image: rust:1.75\n  services:\n    - postgres:15\n  cache:\n    paths: [target]\n  script:\n    - cargo build\n",
            )
            .unwrap();
            let v2 = call(pipeline_capabilities_json(
                rich.as_ptr(),
                k.as_ptr(),
                e.as_ptr(),
            ));
            assert_eq!(v2["overall"], "PossibleWithCaveats", "{v2}");
            let keys: Vec<&str> = v2["features"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f["key"].as_str().unwrap())
                .collect();
            assert!(
                keys.contains(&"service") || keys.contains(&"cache"),
                "rich features: {keys:?}"
            );
        }
    }

    #[test]
    fn capabilities_needs_a_kind() {
        unsafe {
            let y = CString::new("build:\n  script:\n    - x\n").unwrap();
            let e = CString::new("").unwrap();
            let v = call(pipeline_capabilities_json(
                y.as_ptr(),
                e.as_ptr(),
                e.as_ptr(),
            ));
            assert!(v["error"].is_string());
        }
    }

    #[test]
    fn detect_kind_recognises_common_shapes() {
        assert_eq!(detect_kind("kind: pipeline\nsteps:\n  - name: x"), "drone");
        assert_eq!(detect_kind("on:\n  push:\njobs:\n  build:"), "github");
    }
}
