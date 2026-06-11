//! Recipe registry: discover recipes from sources — the bundled **standard
//! library** (embedded at build time, always available) plus any number of
//! **user-configured directories** (local clones of recipe repos) — and list,
//! search, sort and look them up. The standard set lives in `recipes/` at the
//! repo root; that directory is the canonical, publishable recipe repo, and its
//! files are embedded here so a deployed binary ships them with no on-disk
//! dependency.

use std::path::{Path, PathBuf};

use crate::{load, Recipe, RecipeError};

/// The standard recipe library, embedded at build time. To add a standard
/// recipe: drop a `<id>.recipe.yml` in `recipes/` and add one line here.
const STANDARD: &[&str] = &[
    include_str!("../../../recipes/rust-ci.recipe.yml"),
    include_str!("../../../recipes/node-test.recipe.yml"),
    include_str!("../../../recipes/docker-publish.recipe.yml"),
    include_str!("../../../recipes/python-ci.recipe.yml"),
    include_str!("../../../recipes/go-ci.recipe.yml"),
];

/// Where a registered recipe came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipeSource {
    /// Bundled with the tool — the standard library.
    Standard,
    /// A user-configured source (directory / repo), identified by a label.
    User(String),
}

impl RecipeSource {
    /// A short label for display.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Standard => "standard",
            Self::User(name) => name,
        }
    }
}

/// One recipe in the registry, with its origin.
#[derive(Debug, Clone)]
pub struct RecipeEntry {
    pub recipe: Recipe,
    pub source: RecipeSource,
}

/// How to order a recipe listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    /// Alphabetical by recipe id.
    Name,
    /// By first tag, then id (groups related recipes).
    Tag,
    /// Standard library first, then user sources; id within each.
    Source,
}

/// A searchable, sortable collection of recipes from one or more sources.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    entries: Vec<RecipeEntry>,
}

impl Registry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry pre-loaded with the embedded standard library.
    #[must_use]
    pub fn with_standard() -> Self {
        let mut r = Self::new();
        for src in STANDARD {
            if let Ok(recipe) = load(src) {
                r.entries.push(RecipeEntry {
                    recipe,
                    source: RecipeSource::Standard,
                });
            }
        }
        r
    }

    /// Load every `*.recipe.yml` directly under `dir`, tagging each with a
    /// `User(label)` source. Returns the per-file parse errors (malformed files
    /// are skipped, never abort the load) so a caller can surface them.
    ///
    /// # Errors
    /// Never returns `Err` for the load itself; per-file failures are in the Vec.
    pub fn load_dir(&mut self, dir: &Path, label: &str) -> Vec<(PathBuf, RecipeError)> {
        let mut errors = Vec::new();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return errors;
        };
        let mut paths: Vec<PathBuf> = rd
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().ends_with(".recipe.yml"))
            .collect();
        paths.sort(); // deterministic load order
        for path in paths {
            match std::fs::read_to_string(&path) {
                Ok(src) => match load(&src) {
                    Ok(recipe) => self.entries.push(RecipeEntry {
                        recipe,
                        source: RecipeSource::User(label.to_string()),
                    }),
                    Err(e) => errors.push((path, e)),
                },
                Err(e) => errors.push((path, RecipeError::Parse(e.to_string()))),
            }
        }
        errors
    }

    /// All registered recipes, in insertion order.
    #[must_use]
    pub fn all(&self) -> &[RecipeEntry] {
        &self.entries
    }

    /// The recipe with this id, if registered (first match wins — a user source
    /// loaded after standard can shadow by re-using an id).
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&RecipeEntry> {
        self.entries.iter().find(|e| e.recipe.recipe_id == id)
    }

    /// Recipes matching `query` (case-insensitive substring of the id,
    /// description, or any tag). An empty query matches everything. Insertion
    /// order is preserved; use [`Registry::browse`] to also sort.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<&RecipeEntry> {
        let q = query.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|e| matches(&e.recipe, &q))
            .collect()
    }

    /// All recipes ordered by `key`.
    #[must_use]
    pub fn sorted(&self, key: SortKey) -> Vec<&RecipeEntry> {
        let mut out: Vec<&RecipeEntry> = self.entries.iter().collect();
        out.sort_by(|a, b| compare(a, b, key));
        out
    }

    /// The registry view a browser needs: recipes matching `query`, ordered by
    /// `key`. Combines [`Registry::search`] and [`Registry::sorted`].
    #[must_use]
    pub fn browse(&self, query: &str, key: SortKey) -> Vec<&RecipeEntry> {
        let mut out = self.search(query);
        out.sort_by(|a, b| compare(a, b, key));
        out
    }
}

/// Whether a recipe matches a (already lower-cased, trimmed) query. Empty
/// query matches everything.
fn matches(r: &Recipe, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    r.recipe_id.to_lowercase().contains(q)
        || r.description.to_lowercase().contains(q)
        || r.tags.iter().any(|t| t.to_lowercase().contains(q))
}

/// Order two entries by `key`, always tie-breaking on id for stability.
fn compare(a: &RecipeEntry, b: &RecipeEntry, key: SortKey) -> std::cmp::Ordering {
    match key {
        SortKey::Name => a.recipe.recipe_id.cmp(&b.recipe.recipe_id),
        SortKey::Tag => a
            .recipe
            .tags
            .first()
            .cmp(&b.recipe.tags.first())
            .then_with(|| a.recipe.recipe_id.cmp(&b.recipe.recipe_id)),
        SortKey::Source => source_rank(&a.source)
            .cmp(&source_rank(&b.source))
            .then_with(|| a.recipe.recipe_id.cmp(&b.recipe.recipe_id)),
    }
}

impl SortKey {
    /// Parse a sort key from a UI string ("name" | "tag" | "source"); defaults
    /// to [`SortKey::Name`] for anything else.
    #[must_use]
    pub fn from_str_or_name(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "tag" => Self::Tag,
            "source" => Self::Source,
            _ => Self::Name,
        }
    }
}

/// Standard sorts before user; user sources alphabetical by label.
fn source_rank(s: &RecipeSource) -> (u8, String) {
    match s {
        RecipeSource::Standard => (0, String::new()),
        RecipeSource::User(l) => (1, l.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_library_loads() {
        let r = Registry::with_standard();
        assert_eq!(r.all().len(), 5, "five bundled standard recipes");
        assert!(r.get("rust-ci").is_some());
        assert_eq!(r.get("rust-ci").unwrap().source, RecipeSource::Standard);
    }

    #[test]
    fn search_matches_id_description_and_tags() {
        let r = Registry::with_standard();
        assert!(r
            .search("rust")
            .iter()
            .any(|e| e.recipe.recipe_id == "rust-ci"));
        // description match ("Rust crate"): rust-ci again
        assert!(!r.search("crate").is_empty());
        // empty query → everything
        assert_eq!(r.search("").len(), 5);
        // miss
        assert!(r.search("zzz-nonexistent").is_empty());
    }

    #[test]
    fn sorted_by_name_is_alphabetical() {
        let r = Registry::with_standard();
        let ids: Vec<&str> = r
            .sorted(SortKey::Name)
            .iter()
            .map(|e| e.recipe.recipe_id.as_str())
            .collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn user_dir_recipes_are_additive_and_labelled() {
        let dir = std::env::temp_dir().join("pipewright-recipe-test");
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("custom.recipe.yml");
        std::fs::write(&f, "recipe_id: my-custom\ndescription: a user recipe\ntags: [local]\njobs:\n  build:\n    steps:\n      - make\n").unwrap();
        let mut r = Registry::with_standard();
        let errs = r.load_dir(&dir, "my-team");
        assert!(errs.is_empty(), "clean load: {errs:?}");
        let entry = r.get("my-custom").expect("user recipe present");
        assert_eq!(entry.source, RecipeSource::User("my-team".to_string()));
        assert!(r
            .search("local")
            .iter()
            .any(|e| e.recipe.recipe_id == "my-custom"));
        let _ = std::fs::remove_file(&f);
    }
}
