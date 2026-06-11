//! Jenkinsfile (Groovy DSL) render — block-style `key { ... }`.
//!
//! Jenkins declarative pipelines look like nested Groovy blocks:
//!   pipeline {
//!       agent any
//!       stages {
//!           stage('build') { steps { sh 'cargo build' } }
//!       }
//!   }
//!
//! The catalog `jenkins.toml` types each construct as a section;
//! we walk the `serde_yaml::Value` tree and emit `key { … }` for
//! mapping values and `key <arg>` for scalar values. Where the
//! Jenkins DSL takes a single positional argument (`stage('build')`,
//! `sh 'cargo build'`) we render the string in quotes after the
//! block name.

use serde_yaml::{Mapping, Value};
use std::fmt::Write;

pub fn render(v: &Value) -> Result<String, String> {
    let Value::Mapping(m) = v else {
        return Err("jenkinsfile root must be a mapping".into());
    };
    let mut out = String::new();
    // The catalog's root is the `pipeline` section. Wrap it.
    out.push_str("pipeline {\n");
    emit_block(m, 4, &mut out);
    out.push_str("}\n");
    Ok(out)
}

fn emit_block(m: &Mapping, indent: usize, out: &mut String) {
    for (k, v) in m {
        let key = scalar_str(k);
        if key.is_empty() {
            continue;
        }
        write_indent(indent, out);
        match v {
            Value::Mapping(inner) => {
                if inner.is_empty() {
                    let _ = writeln!(out, "{key} {{}}");
                } else {
                    let _ = writeln!(out, "{key} {{");
                    emit_block(inner, indent + 4, out);
                    write_indent(indent, out);
                    out.push_str("}\n");
                }
            }
            Value::Sequence(items) => {
                if items.is_empty() {
                    let _ = writeln!(out, "{key} {{}}");
                } else {
                    // Item list. Each MAPPING item is a construct element
                    // rendered in the Jenkins idiom `<construct>('<name>') {
                    // body }` (matching emit_jenkinsfile exactly, so the
                    // round-trip seeds an identical hub); a SCALAR item is a
                    // bare string literal (`libraries { 'lib' }`). The old
                    // flatten (`stages { name 'go' … }`) desynced from emit.
                    let construct = singularize(&key);
                    let _ = writeln!(out, "{key} {{");
                    for item in items {
                        match item {
                            Value::Mapping(im) => {
                                let name = im
                                    .get(Value::String("name".into()))
                                    .map(scalar_str)
                                    .filter(|s| !s.is_empty());
                                write_indent(indent + 4, out);
                                if let Some(n) = name {
                                    let _ = writeln!(
                                        out,
                                        "{construct}('{}') {{",
                                        n.replace('\'', "\\'")
                                    );
                                } else {
                                    let _ = writeln!(out, "{construct} {{");
                                }
                                let body: Mapping = im
                                    .iter()
                                    .filter(|(k, _)| scalar_str(k) != "name")
                                    .map(|(k, v)| (k.clone(), v.clone()))
                                    .collect();
                                emit_block(&body, indent + 8, out);
                                write_indent(indent + 4, out);
                                out.push_str("}\n");
                            }
                            other => {
                                write_indent(indent + 4, out);
                                let _ = writeln!(out, "{}", scalar_inline(other));
                            }
                        }
                    }
                    write_indent(indent, out);
                    out.push_str("}\n");
                }
            }
            Value::String(s) if s.is_empty() => {
                let _ = writeln!(out, "{key}");
            }
            other => {
                let _ = writeln!(out, "{key} {}", scalar_inline(other));
            }
        }
    }
}

fn scalar_inline(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("'{}'", s.replace('\'', "\\'")),
        _ => String::new(),
    }
}

fn scalar_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Singular construct keyword for an item-list field key — the Jenkins idiom
/// renders `stages { stage(…) }`, `parameters { parameter(…) }`,
/// `triggers { trigger(…) }`. Matches the seeder's classify construct (which
/// strips the same trailing `s`), so render and emit agree on the keyword.
fn singularize(key: &str) -> String {
    key.strip_suffix('s').unwrap_or(key).to_string()
}

fn write_indent(n: usize, out: &mut String) {
    for _ in 0..n {
        out.push(' ');
    }
}
