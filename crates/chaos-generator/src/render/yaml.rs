//! YAML renderer for chaos values.
//!
//! Custom emitter — walks `serde_yaml::Value` and writes block-
//! style YAML with indented sequences (`steps:\n  - item`).
//! serde_yaml's own to_string emits unindented sequences which
//! pipeline-cst's parser then misreads as a Scalar value, dropping
//! every step from the seeded CST. The chaos roundtrip would
//! silently pass on empty IR if we used the default emitter.

use serde_yaml::{Mapping, Value};
use std::fmt::Write;

pub fn render(v: &Value) -> Result<String, String> {
    let mut out = String::new();
    emit(v, 0, &mut out);
    Ok(out)
}

fn emit(v: &Value, indent: usize, out: &mut String) {
    match v {
        Value::Mapping(m) => emit_mapping_top(m, indent, out),
        Value::Sequence(s) => emit_seq_top(s, indent, out),
        _ => {
            write_scalar(v, out);
            out.push('\n');
        }
    }
}

fn emit_mapping_top(m: &Mapping, indent: usize, out: &mut String) {
    if m.is_empty() {
        write_indent(indent, out);
        out.push_str("{}\n");
        return;
    }
    for (k, v) in m {
        write_indent(indent, out);
        let key = match k {
            Value::String(s) => s.clone(),
            other => render_scalar_inline(other),
        };
        out.push_str(&yaml_key(&key));
        out.push(':');
        match v {
            Value::Mapping(inner) if inner.is_empty() => {
                out.push_str(" {}\n");
            }
            Value::Sequence(s) if s.is_empty() => {
                out.push_str(" []\n");
            }
            Value::Mapping(inner) => {
                out.push('\n');
                emit_mapping_top(inner, indent + 2, out);
            }
            Value::Sequence(s) => {
                out.push('\n');
                emit_seq_top(s, indent + 2, out);
            }
            other => {
                out.push(' ');
                write_scalar(other, out);
                out.push('\n');
            }
        }
    }
}

fn emit_seq_top(s: &[Value], indent: usize, out: &mut String) {
    if s.is_empty() {
        write_indent(indent, out);
        out.push_str("[]\n");
        return;
    }
    for item in s {
        write_indent(indent, out);
        out.push_str("- ");
        match item {
            Value::Mapping(m) if m.is_empty() => {
                out.push_str("{}\n");
            }
            Value::Sequence(inner) if inner.is_empty() => {
                out.push_str("[]\n");
            }
            Value::Mapping(m) => {
                // First entry of the mapping rides on the same
                // line as the dash; subsequent entries get the
                // dash's content indent (= indent + 2).
                let mut first = true;
                for (k, v) in m {
                    if !first {
                        write_indent(indent + 2, out);
                    }
                    first = false;
                    let key = match k {
                        Value::String(s) => s.clone(),
                        other => render_scalar_inline(other),
                    };
                    out.push_str(&yaml_key(&key));
                    out.push(':');
                    match v {
                        Value::Mapping(inner) if inner.is_empty() => {
                            out.push_str(" {}\n");
                        }
                        Value::Sequence(s) if s.is_empty() => {
                            out.push_str(" []\n");
                        }
                        Value::Mapping(inner) => {
                            out.push('\n');
                            emit_mapping_top(inner, indent + 4, out);
                        }
                        Value::Sequence(s) => {
                            out.push('\n');
                            emit_seq_top(s, indent + 4, out);
                        }
                        other => {
                            out.push(' ');
                            write_scalar(other, out);
                            out.push('\n');
                        }
                    }
                }
            }
            Value::Sequence(inner) => {
                out.push('\n');
                emit_seq_top(inner, indent + 4, out);
            }
            other => {
                write_scalar(other, out);
                out.push('\n');
            }
        }
    }
}

fn write_indent(n: usize, out: &mut String) {
    for _ in 0..n {
        out.push(' ');
    }
}

fn write_scalar(v: &Value, out: &mut String) {
    out.push_str(&render_scalar_inline(v));
}

fn render_scalar_inline(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => quote_if_needed(s),
        Value::Sequence(_) | Value::Mapping(_) => {
            // Best-effort flow form for nested values that can't
            // be block-emitted here.
            serde_yaml::to_string(v)
                .unwrap_or_default()
                .trim()
                .to_string()
        }
        Value::Tagged(_) => serde_yaml::to_string(v).unwrap_or_default(),
    }
}

fn yaml_key(s: &str) -> String {
    // Plain keys are OK if they don't contain special chars. We
    // mirror the conservative quoting from `crate::emit` so the
    // generator and reverse-emit agree on key escaping.
    if s.is_empty()
        || s.chars()
            .any(|c| matches!(c, ':' | '#' | '\n' | '[' | ']' | '{' | '}'))
        || s.starts_with(['-', '?'])
    {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn quote_if_needed(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    let problematic = s.contains([':', '#', '\n'])
        || s.starts_with(['-', '?', '!', '|', '>', '*', '&', '%', '@', '`']);
    let looks_bool_or_null = matches!(
        s,
        "true"
            | "false"
            | "null"
            | "yes"
            | "no"
            | "on"
            | "off"
            | "True"
            | "False"
            | "Null"
            | "Yes"
            | "No"
    );
    let looks_number = s.parse::<f64>().is_ok();
    if problematic || looks_bool_or_null || looks_number {
        let mut s2 = String::with_capacity(s.len() + 2);
        s2.push('\'');
        for c in s.chars() {
            if c == '\'' {
                s2.push('\'');
            }
            s2.push(c);
        }
        s2.push('\'');
        s2
    } else {
        s.to_string()
    }
}

// Suppress unused-imports lint in the closure-less version.
#[allow(dead_code)]
fn _unused(_: &mut String) {
    let _ = write!(String::new(), "");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unindented_dash_gets_two_spaces() {
        let yaml = render(&serde_yaml::from_str("steps:\n- item\n- other\n").unwrap()).unwrap();
        assert!(yaml.contains("  - item"), "got: {yaml}");
        assert!(yaml.contains("  - other"), "got: {yaml}");
    }

    #[test]
    fn nested_dash_blocks_indented() {
        let yaml = render(&serde_yaml::from_str("outer:\n- inner:\n  - sub\n").unwrap()).unwrap();
        assert!(yaml.contains("  - inner:"), "got: {yaml}");
        assert!(yaml.contains("      - sub"), "got: {yaml}");
    }
}
