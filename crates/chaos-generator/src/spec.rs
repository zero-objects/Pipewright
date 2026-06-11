//! Catalog TOML → typed schema.
//!
//! Each `catalog/<platform>.toml` lists every platform key with a
//! type string. We parse those types into [`Type`] so the random
//! walker can decide what to emit (a scalar, a list-of-strings,
//! a recursion into another section, …). The same grammar that
//! drives `gen_ruleset.py` drives the chaos generator: there is no
//! second source of truth.

use indexmap::IndexMap;
use std::path::Path;

/// One whole platform inventory.
#[derive(Debug, Clone)]
pub struct Spec {
    pub platform: String,
    /// Section name (e.g. `kind_pipeline`, `step`) → its fields.
    pub sections: IndexMap<String, Section>,
}

/// One construct section (a TOML `[<name>]` table).
#[derive(Debug, Clone)]
pub struct Section {
    pub name: String,
    /// Insertion-ordered so generated YAML keeps key order stable.
    pub fields: IndexMap<String, Field>,
}

/// One field inside a section.
#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    pub required: bool,
    pub options: Vec<String>,
}

/// Field type as written in the TOML.
///
/// Bare construct names (e.g. `concurrency`, `platform`) are
/// recorded as [`Type::Section`] and resolved by the walker by
/// looking up the section in the same [`Spec`]. Unknown bare
/// names fall back to scalar-string (the catalog is hand-authored;
/// any string we don't recognise as a structural pointer is treated
/// as a leaf).
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    String,
    Number,
    Integer,
    Boolean,
    Null,
    Any,
    Secret,
    Object,
    Enum,
    List(Box<Type>),
    Map(Box<Type>),
    Union(Vec<Type>),
    Section(String),
}

#[derive(Debug, thiserror::Error)]
pub enum SpecError {
    #[error("read {0}: {1}")]
    Read(String, std::io::Error),
    #[error("parse {0}: {1}")]
    Parse(String, toml::de::Error),
    #[error("type `{ty}` for {section}.{field}: {reason}")]
    Type {
        section: String,
        field: String,
        ty: String,
        reason: String,
    },
}

/// Load a catalog inventory from disk.
pub fn load(path: &Path) -> Result<Spec, SpecError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| SpecError::Read(path.display().to_string(), e))?;
    let table: toml::Table =
        toml::from_str(&raw).map_err(|e| SpecError::Parse(path.display().to_string(), e))?;

    let mut platform = String::new();
    let mut sections: IndexMap<String, Section> = IndexMap::new();

    for (section_name, section_val) in &table {
        let toml::Value::Table(body) = section_val else {
            continue;
        };
        if section_name == "meta" {
            if let Some(toml::Value::String(p)) = body.get("platform") {
                platform = p.clone();
            }
            continue;
        }
        let mut fields: IndexMap<String, Field> = IndexMap::new();
        for (field_name, field_val) in body {
            let toml::Value::Table(spec) = field_val else {
                continue;
            };
            let Some(toml::Value::String(ty_str)) = spec.get("type") else {
                continue;
            };
            let required = matches!(spec.get("required"), Some(toml::Value::Boolean(true)));
            let options: Vec<String> = spec
                .get("options")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let ty = parse_type(ty_str).map_err(|reason| SpecError::Type {
                section: section_name.clone(),
                field: field_name.clone(),
                ty: ty_str.clone(),
                reason,
            })?;
            fields.insert(
                field_name.clone(),
                Field {
                    name: field_name.clone(),
                    ty,
                    required,
                    options,
                },
            );
        }
        sections.insert(
            section_name.clone(),
            Section {
                name: section_name.clone(),
                fields,
            },
        );
    }

    if platform.is_empty() {
        platform = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
    }

    Ok(Spec { platform, sections })
}

/// Parse a type string like `"list<string> | string"` or
/// `"map<boolean | number | secret | string>"`.
pub fn parse_type(s: &str) -> Result<Type, String> {
    let s = s.trim();
    let arms = top_split(s, '|');
    if arms.len() > 1 {
        let mut variants = Vec::with_capacity(arms.len());
        for arm in arms {
            variants.push(parse_type(arm.trim())?);
        }
        return Ok(Type::Union(variants));
    }
    if let Some(inner) = strip_wrapper(s, "list") {
        return Ok(Type::List(Box::new(parse_type(inner)?)));
    }
    if let Some(inner) = strip_wrapper(s, "map") {
        return Ok(Type::Map(Box::new(parse_type(inner)?)));
    }
    Ok(match s {
        "string" => Type::String,
        "number" => Type::Number,
        "integer" => Type::Integer,
        "boolean" => Type::Boolean,
        "null" => Type::Null,
        "any" => Type::Any,
        "secret" => Type::Secret,
        "object" => Type::Object,
        "enum" => Type::Enum,
        other if !other.is_empty() => Type::Section(other.to_string()),
        _ => return Err("empty type".into()),
    })
}

/// Split at depth-0 occurrences of `sep` (ignores nested `<...>`).
fn top_split(s: &str, sep: char) -> Vec<&str> {
    let mut depth: i32 = 0;
    let mut last = 0usize;
    let mut out = Vec::new();
    for (i, ch) in s.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push(&s[last..i]);
                last = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[last..]);
    out
}

/// `strip_wrapper("list<string>", "list")` → `Some("string")`.
fn strip_wrapper<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{name}<");
    let inner = s.strip_prefix(&prefix)?.strip_suffix('>')?;
    Some(inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars() {
        assert_eq!(parse_type("string").unwrap(), Type::String);
        assert_eq!(parse_type("boolean").unwrap(), Type::Boolean);
    }

    #[test]
    fn list_inner() {
        assert_eq!(
            parse_type("list<string>").unwrap(),
            Type::List(Box::new(Type::String))
        );
    }

    #[test]
    fn nested_map() {
        let t = parse_type("map<boolean | number | secret | string>").unwrap();
        let Type::Map(inner) = t else { panic!() };
        let Type::Union(arms) = *inner else { panic!() };
        assert_eq!(arms.len(), 4);
    }

    #[test]
    fn union_with_list() {
        let t = parse_type("list<string> | string").unwrap();
        let Type::Union(arms) = t else { panic!() };
        assert_eq!(arms.len(), 2);
        assert!(matches!(arms[0], Type::List(_)));
        assert_eq!(arms[1], Type::String);
    }

    #[test]
    fn section_reference() {
        assert_eq!(
            parse_type("concurrency").unwrap(),
            Type::Section("concurrency".into())
        );
    }
}
