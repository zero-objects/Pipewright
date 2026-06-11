//! `pipeline` — the command-line frontend over the pipeline library.
//!
//! Wraps the same engine the Qt6 UI uses (forward cascade, renderer, migration,
//! recipes) so pipelines can be inspected, rendered, migrated, composed and run
//! from a shell or CI. Platform is auto-detected from the source unless given
//! with `-p/--platform`. Most commands print to stdout; errors go to stderr and
//! set a non-zero exit code.

mod run;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "pipewright",
    version,
    about = "Losslessly translate, inspect, and run CI/CD pipelines across 17 platforms."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Detect the platform of a pipeline file.
    Detect { file: PathBuf },
    /// List the platforms the engine supports.
    Platforms,
    /// Print the pipeline as structured JSON (`{pipeline:{jobs:[…]}}`).
    Inspect {
        file: PathBuf,
        #[arg(short, long)]
        platform: Option<String>,
    },
    /// Render the pipeline (diagram SVG, runbook, or inspect JSON).
    Render {
        file: PathBuf,
        #[arg(short, long)]
        platform: Option<String>,
        #[arg(long, value_enum, default_value_t = RenderFormat::Text)]
        format: RenderFormat,
        /// Prose language for text/markdown/html runbooks.
        #[arg(long, default_value = "en")]
        locale: String,
    },
    /// Migrate a pipeline to another platform.
    Migrate {
        file: PathBuf,
        #[arg(short, long)]
        platform: Option<String>,
        #[arg(long)]
        to: String,
    },
    /// Compose one or more recipe files into a pipeline for a target platform.
    Compose {
        recipes: Vec<PathBuf>,
        #[arg(long)]
        to: String,
    },
    /// Apply a recipe file to a pipeline (merge its jobs in) and re-emit.
    Apply {
        recipe: PathBuf,
        /// Existing pipeline to merge into; omit to start fresh.
        #[arg(long)]
        into: Option<PathBuf>,
        /// Target platform (defaults to the `--into` pipeline's platform, else gitlab).
        #[arg(long)]
        to: Option<String>,
    },
    /// List the standard recipe library (id, version, source, description).
    Recipes {
        /// Filter by id / description / tag substring.
        #[arg(long, default_value = "")]
        query: String,
    },
    /// Print a generated, localized description of a recipe.
    Describe {
        /// A library recipe id (as listed by `recipes`) or a recipe file path.
        recipe: String,
        #[arg(long, default_value = "en")]
        locale: String,
    },
    /// Print the pipeline's capability profile (features used + portability hint).
    Capabilities {
        file: PathBuf,
        #[arg(short, long)]
        platform: Option<String>,
    },
    /// Print a plan of what `run` would execute (jobs in dependency order, no Docker).
    Plan {
        file: PathBuf,
        #[arg(short, long)]
        platform: Option<String>,
    },
    /// Run the pipeline locally in Docker, streaming each job's output.
    Run {
        file: PathBuf,
        #[arg(short, long)]
        platform: Option<String>,
        /// Only run this job (and skip the rest).
        #[arg(long)]
        job: Option<String>,
        /// Trigger event the run simulates — drives `rules:if` evaluation
        /// (push / tag / `merge_request` / schedule / manual / external).
        #[arg(long, default_value = "push")]
        trigger: String,
        /// Git ref name (branch for push, tag name for tag).
        #[arg(long, default_value = "main")]
        r#ref: String,
        /// Mount the workspace read-write so the pipeline's commands can modify
        /// it. Only for pipelines you trust — by default the mount is
        /// read-only. (For a safe writable run, use --rw-copy.)
        #[arg(long, conflicts_with = "rw_copy")]
        rw: bool,
        /// Mount a throwaway COPY of the workspace read-write: builds that write
        /// (e.g. `target/`) work, but the real directory is never touched.
        #[arg(long)]
        rw_copy: bool,
    },
}

#[derive(Copy, Clone, ValueEnum)]
enum RenderFormat {
    /// Plain-text runbook prose.
    Text,
    /// Markdown runbook.
    Md,
    /// HTML runbook JSON.
    Html,
    /// Diagram SVG.
    Svg,
    /// Structured pipeline JSON.
    Json,
}

fn main() -> ExitCode {
    match run_command(Cli::parse().command) {
        Ok(out) => {
            if !out.is_empty() {
                println!("{out}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Read a file, mapping IO errors to a message.
fn read(path: &PathBuf) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Resolve the platform: explicit `-p`, else auto-detect — preferring the file
/// NAME (`.gitlab-ci.yml`, `Jenkinsfile`, …) over the content when available.
fn resolve_platform(
    explicit: Option<&str>,
    src: &str,
    path: Option<&str>,
) -> Result<String, String> {
    if let Some(p) = explicit {
        if !pipeline_forward::is_supported(p) {
            return Err(format!("unknown platform '{p}' (try `pipeline platforms`)"));
        }
        return Ok(p.to_string());
    }
    let detected = match path {
        Some(p) => pipeline_forward::detect_with_path(p, src),
        None => pipeline_forward::detect(src),
    };
    detected
        .map(ToString::to_string)
        .ok_or_else(|| "could not detect platform — pass -p/--platform".to_string())
}

/// Forward + lift a pipeline from a file.
fn lift(
    file: &PathBuf,
    platform: Option<&str>,
) -> Result<(String, pipeline_render::Pipeline), String> {
    let src = read(file)?;
    let plat = resolve_platform(platform, &src, file.to_str())?;
    let g = pipeline_forward::forward(&plat, &src).map_err(|e| e.to_string())?;
    let p = pipeline_render::lift(&g).ok_or_else(|| "the IR holds no pipeline".to_string())?;
    Ok((plat, p))
}

#[allow(clippy::too_many_lines)]
fn run_command(cmd: Command) -> Result<String, String> {
    match cmd {
        Command::Detect { file } => {
            let src = read(&file)?;
            Ok(
                pipeline_forward::detect_with_path(file.to_str().unwrap_or(""), &src)
                    .unwrap_or("unknown")
                    .to_string(),
            )
        }
        Command::Platforms => Ok(pipeline_forward::PLATFORMS.join("\n")),
        Command::Inspect { file, platform } => {
            let (_, p) = lift(&file, platform.as_deref())?;
            Ok(pipeline_render::inspect_json(&p))
        }
        Command::Render {
            file,
            platform,
            format,
            locale,
        } => {
            let (_, p) = lift(&file, platform.as_deref())?;
            Ok(match format {
                RenderFormat::Text => pipeline_render::describe(&p),
                RenderFormat::Md => pipeline_render::markdown_in(&p, &locale),
                RenderFormat::Html => pipeline_render::runbook_json_in(&p, &locale),
                RenderFormat::Svg => pipeline_render::render_diagram(&p).svg,
                RenderFormat::Json => pipeline_render::inspect_json(&p),
            })
        }
        Command::Migrate { file, platform, to } => {
            let src = read(&file)?;
            let from = resolve_platform(platform.as_deref(), &src, file.to_str())?;
            let (out, report) = pipeline_forward::migrate_with_report(&from, &src, &to)
                .map_err(|e| e.to_string())?;
            if out.trim().is_empty() || out.trim() == "{}" {
                return Err(format!(
                    "'{from}' → '{to}' produced nothing (incompatible platform structures)"
                ));
            }
            // The migrated YAML goes to stdout (pipeable); the friction report
            // goes to stderr so it never corrupts the output but is never
            // silent — the Roast's core complaint.
            if report.is_empty() {
                eprintln!("friction: none — every capability the source uses survived to {to}.");
            } else {
                eprintln!("friction report ({} item(s)):", report.len());
                for f in &report {
                    let mark = match f.severity {
                        "info" => "i",
                        "approximated" => "~",
                        _ => "!",
                    };
                    eprintln!("  [{mark}] {}: {}", f.severity, f.note);
                }
            }
            Ok(out)
        }
        Command::Compose { recipes, to } => {
            if recipes.is_empty() {
                return Err("no recipe files given".to_string());
            }
            let docs: Result<Vec<String>, _> = recipes.iter().map(read).collect();
            pipeline_recipe::compose_documents(&docs?, &to).map_err(|e| e.to_string())
        }
        Command::Apply { recipe, into, to } => {
            let recipe = pipeline_recipe::load(&read(&recipe)?).map_err(|e| e.to_string())?;
            let (existing, kind) = match &into {
                Some(f) => {
                    let src = read(f)?;
                    let k = to
                        .clone()
                        .map_or_else(|| resolve_platform(None, &src, f.to_str()), Ok)?;
                    (src, k)
                }
                None => (
                    String::new(),
                    to.clone().unwrap_or_else(|| "gitlab".to_string()),
                ),
            };
            pipeline_recipe::apply_to_source(&existing, &kind, &recipe).map_err(|e| e.to_string())
        }
        Command::Recipes { query } => {
            use pipeline_recipe::registry::{Registry, SortKey};
            use std::fmt::Write as _;
            let reg = Registry::with_standard();
            let mut out = String::new();
            for e in reg.browse(&query, SortKey::Name) {
                let r = &e.recipe;
                let ver = if r.recipe_version.is_empty() {
                    "-"
                } else {
                    &r.recipe_version
                };
                let _ = writeln!(
                    out,
                    "{:<16} {:<8} {:<10} {}",
                    r.recipe_id,
                    ver,
                    e.source.label(),
                    r.description
                );
            }
            Ok(out.trim_end().to_string())
        }
        Command::Describe { recipe, locale } => {
            // Library id first (the ids `recipes` lists), file path second —
            // so the obvious `recipes` → `describe <id>` flow works, and
            // local recipe files still do too.
            let reg = pipeline_recipe::registry::Registry::with_standard();
            let loaded = match reg.get(&recipe) {
                Some(entry) => entry.recipe.clone(),
                None => pipeline_recipe::load(&read(&PathBuf::from(&recipe))?)
                    .map_err(|e| e.to_string())?,
            };
            pipeline_recipe::describe_recipe(&loaded, &locale).map_err(|e| e.to_string())
        }
        Command::Capabilities { file, platform } => {
            let src = read(&file)?;
            let plat = resolve_platform(platform.as_deref(), &src, file.to_str())?;
            let g = pipeline_forward::forward(&plat, &src).map_err(|e| e.to_string())?;
            Ok(pipeline_render::capabilities_json(&g))
        }
        Command::Plan { file, platform } => {
            use std::fmt::Write as _;
            let (plat, p) = lift(&file, platform.as_deref())?;
            let mut out = String::new();
            // A plan for a translate-only platform would show jobs with no
            // commands — say why up front instead of a misleading empty plan.
            if pipeline_forward::run_support(&plat) != pipeline_forward::RunSupport::Full {
                let _ = writeln!(
                    out,
                    "note: {plat} pipelines are translate/inspect-only locally — {}\n",
                    pipeline_forward::RunSupport::reason(&plat)
                );
            }
            out.push_str(&run::plan(&p));
            Ok(out)
        }
        Command::Run {
            file,
            platform,
            job,
            trigger,
            r#ref,
            rw,
            rw_copy,
        } => {
            let (plat, p) = lift(&file, platform.as_deref())?;
            // Don't pretend to run a pipeline whose work isn't local shell.
            if pipeline_forward::run_support(&plat) != pipeline_forward::RunSupport::Full {
                return Err(format!(
                    "{plat} pipelines can't run locally: {}\n\
                     (use `inspect`, `render`, or `migrate` instead.)",
                    pipeline_forward::RunSupport::reason(&plat)
                ));
            }
            // The workspace bind-mounted into every job is the directory the
            // pipeline file lives in (the repo root, conventionally).
            let workspace = file
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                .map_or_else(
                    || std::path::PathBuf::from("."),
                    std::path::Path::to_path_buf,
                );
            let trig = run::Trigger {
                event: trigger,
                r#ref,
            };
            // Secure by default: read-only mount unless the user opts into
            // writes (--rw in place, or --rw-copy on a throwaway clone).
            let mount = if rw {
                run::MountMode::ReadWrite
            } else if rw_copy {
                run::MountMode::ReadWriteCopy
            } else {
                run::MountMode::ReadOnly
            };
            run::run(&p, job.as_deref(), &workspace, &trig, mount)
        }
    }
}
