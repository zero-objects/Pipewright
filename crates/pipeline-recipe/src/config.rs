//! Recipe-source configuration: where recipes come from beyond the embedded
//! standard library. A user declares any number of sources in a small YAML
//! file; each is either a **local directory** of `*.recipe.yml` files (a clone
//! the user manages) or a **git repository** that is cloned/updated into a
//! local cache and then loaded. This is what backs "user-defined repos": the
//! standard library is always present, and these add to it.
//!
//! ```yaml
//! sources:
//!   - label: my-team          # a directory you keep in sync yourself
//!     dir: /home/me/ci-recipes
//!   - label: community        # a git repo we clone/pull into the cache
//!     git: https://github.com/example/ci-recipes.git
//!     reference: main         # optional branch/tag/sha; default branch if omitted
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::registry::Registry;
use crate::RecipeError;

/// The full recipe-source configuration, typically parsed from a YAML file.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RecipeConfig {
    /// Additional recipe sources, in load order (a later source can shadow an
    /// id from an earlier one — see [`Registry::get`]).
    #[serde(default)]
    pub sources: Vec<SourceSpec>,
}

/// One configured recipe source: a `label` plus exactly one location — either a
/// local `dir` or a `git` URL (with an optional `reference`).
#[derive(Debug, Clone, Deserialize)]
pub struct SourceSpec {
    /// Display label, and the cache subdirectory name for git sources.
    pub label: String,
    /// A local directory of recipes.
    #[serde(default)]
    pub dir: Option<PathBuf>,
    /// A git repository URL to clone/update.
    #[serde(default)]
    pub git: Option<String>,
    /// For a git source: the branch/tag/commit to check out. Default branch if
    /// omitted.
    #[serde(default)]
    pub reference: Option<String>,
}

impl SourceSpec {
    /// Validate that exactly one location is set.
    ///
    /// # Errors
    /// [`RecipeError::Source`] if neither or both of `dir`/`git` are present.
    fn validate(&self) -> Result<(), RecipeError> {
        match (self.dir.is_some(), self.git.is_some()) {
            (true, false) | (false, true) => Ok(()),
            (false, false) => Err(RecipeError::Source(format!(
                "source '{}' sets neither 'dir' nor 'git'",
                self.label
            ))),
            (true, true) => Err(RecipeError::Source(format!(
                "source '{}' sets both 'dir' and 'git' — pick one",
                self.label
            ))),
        }
    }
}

/// Parse a recipe-source configuration from YAML.
///
/// # Errors
/// [`RecipeError::Parse`] on malformed YAML.
pub fn load_config(yaml: &str) -> Result<RecipeConfig, RecipeError> {
    serde_yaml::from_str(yaml).map_err(|e| RecipeError::Parse(e.to_string()))
}

impl Registry {
    /// Extend this registry with every source in `config`. Git sources are
    /// cloned/updated into `cache_dir/<label>` before loading. Each source is
    /// independent: a failure (bad spec, clone failure, parse error) is
    /// collected and returned, never aborts the others, so a partial config
    /// still loads what it can. The returned errors are keyed by source label.
    pub fn load_config(
        &mut self,
        config: &RecipeConfig,
        cache_dir: &Path,
    ) -> Vec<(String, RecipeError)> {
        let mut errors = Vec::new();
        for src in &config.sources {
            if let Err(e) = src.validate() {
                errors.push((src.label.clone(), e));
                continue;
            }
            let dir = if let Some(dir) = &src.dir {
                dir.clone()
            } else {
                // git source — clone/update into the cache, then load that dir.
                let dest = cache_dir.join(&src.label);
                match ensure_repo(
                    src.git.as_deref().unwrap_or_default(),
                    src.reference.as_deref(),
                    &dest,
                ) {
                    Ok(()) => dest,
                    Err(e) => {
                        errors.push((src.label.clone(), e));
                        continue;
                    }
                }
            };
            for (path, e) in self.load_dir(&dir, &src.label) {
                errors.push((
                    src.label.clone(),
                    RecipeError::Parse(format!("{}: {e}", path.display())),
                ));
            }
        }
        errors
    }
}

/// Ensure a git repo is cloned at `dest` and on `reference` (if given). Clones
/// when absent, otherwise fetches and resets to the wanted ref / pulls the
/// current branch. Shells out to `git`.
///
/// # Errors
/// [`RecipeError::Source`] if `git` is unavailable or any git step fails.
fn ensure_repo(url: &str, reference: Option<&str>, dest: &Path) -> Result<(), RecipeError> {
    let exists = dest.join(".git").is_dir();
    if exists {
        run_git(
            dest.parent().unwrap_or(Path::new(".")),
            &["-C", &dest.to_string_lossy(), "fetch", "--quiet"],
        )?;
        if let Some(r) = reference {
            run_git(
                dest,
                &["-C", &dest.to_string_lossy(), "checkout", "--quiet", r],
            )?;
            // best-effort fast-forward of a tracked branch; ignore detached-HEAD failures.
            let _ = Command::new("git")
                .args([
                    "-C",
                    &dest.to_string_lossy(),
                    "pull",
                    "--quiet",
                    "--ff-only",
                ])
                .output();
        } else {
            run_git(
                dest,
                &[
                    "-C",
                    &dest.to_string_lossy(),
                    "pull",
                    "--quiet",
                    "--ff-only",
                ],
            )?;
        }
    } else {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| RecipeError::Source(format!("cannot create cache dir: {e}")))?;
        }
        let mut args = vec!["clone", "--quiet"];
        if let Some(r) = reference {
            args.extend_from_slice(&["--branch", r]);
        }
        args.push(url);
        let dest_str = dest.to_string_lossy().to_string();
        args.push(&dest_str);
        run_git(Path::new("."), &args)?;
    }
    Ok(())
}

/// Run a git command, mapping a missing binary or non-zero exit to a source error.
fn run_git(_cwd: &Path, args: &[&str]) -> Result<(), RecipeError> {
    let out = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| RecipeError::Source(format!("git not available: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(RecipeError::Source(format!(
            "git {} failed: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config_with_both_source_kinds() {
        let cfg = load_config(
            "sources:\n  - label: team\n    dir: /tmp/recipes\n  - label: community\n    git: https://example.com/r.git\n    reference: main\n",
        )
        .expect("parse");
        assert_eq!(cfg.sources.len(), 2);
        assert_eq!(
            cfg.sources[0].dir.as_deref().unwrap().to_str().unwrap(),
            "/tmp/recipes"
        );
        assert_eq!(
            cfg.sources[1].git.as_deref().unwrap(),
            "https://example.com/r.git"
        );
        assert_eq!(cfg.sources[1].reference.as_deref().unwrap(), "main");
    }

    #[test]
    fn a_source_with_neither_or_both_locations_is_an_error() {
        assert!(SourceSpec {
            label: "x".into(),
            dir: None,
            git: None,
            reference: None
        }
        .validate()
        .is_err());
        assert!(SourceSpec {
            label: "x".into(),
            dir: Some("/a".into()),
            git: Some("u".into()),
            reference: None,
        }
        .validate()
        .is_err());
    }

    #[test]
    fn load_config_loads_a_dir_source() {
        let dir = std::env::temp_dir().join("pipewright-config-dir-test");
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("x.recipe.yml");
        std::fs::write(
            &f,
            "recipe_id: cfg-dir\njobs:\n  b:\n    steps:\n      - make\n",
        )
        .unwrap();
        let cfg = RecipeConfig {
            sources: vec![SourceSpec {
                label: "team".into(),
                dir: Some(dir.clone()),
                git: None,
                reference: None,
            }],
        };
        let mut r = Registry::with_standard();
        let errs = r.load_config(&cfg, Path::new("/unused"));
        assert!(errs.is_empty(), "clean load: {errs:?}");
        assert!(r.get("cfg-dir").is_some(), "dir-source recipe loaded");
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn load_config_clones_and_loads_a_git_source() {
        // Prove the git path without a network: build a real local git repo
        // holding one recipe, then point a git source at it via a file path.
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("git not installed — skipping git-source test");
            return;
        }
        let base = std::env::temp_dir().join("pipewright-config-git-test");
        let _ = std::fs::remove_dir_all(&base);
        let origin = base.join("origin");
        let cache = base.join("cache");
        std::fs::create_dir_all(&origin).unwrap();
        let recipe = "recipe_id: cfg-git\ndescription: from a cloned repo\ntags: [cloned]\njobs:\n  b:\n    steps:\n      - make\n";
        std::fs::write(origin.join("x.recipe.yml"), recipe).unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .args(["-C", origin.to_str().unwrap()])
                .args(args)
                .output()
                .unwrap();
        };
        git(&["init", "--quiet"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        git(&["add", "."]);
        git(&["commit", "--quiet", "-m", "init"]);

        let cfg = RecipeConfig {
            sources: vec![SourceSpec {
                label: "community".into(),
                dir: None,
                git: Some(origin.to_string_lossy().to_string()),
                reference: None,
            }],
        };
        let mut r = Registry::with_standard();
        let errs = r.load_config(&cfg, &cache);
        assert!(errs.is_empty(), "clean clone+load: {errs:?}");
        let entry = r.get("cfg-git").expect("cloned recipe present");
        assert_eq!(entry.source.label(), "community");

        // Second load must update in place (clone already exists), not fail.
        let mut r2 = Registry::with_standard();
        let errs2 = r2.load_config(&cfg, &cache);
        assert!(errs2.is_empty(), "re-load (fetch/pull) clean: {errs2:?}");
        assert!(r2.get("cfg-git").is_some());

        let _ = std::fs::remove_dir_all(&base);
    }
}
