//! Earthfile render — verb-prefixed DSL.
//!
//! Earthfile is `VERB args` lines, not `key: value`. The chaos
//! walker produces a `serde_yaml::Value` that's structured per
//! the catalog (top-level keys VERSION/PROJECT/base_recipe/
//! targets), and this renderer flattens it into the DSL form
//! pipeline-earthfile-cst expects.

use serde_yaml::Value;
use std::fmt::Write;

pub fn render(v: &Value) -> Result<String, String> {
    let mut out = String::new();
    let Value::Mapping(m) = v else {
        return Err("earthfile root must be a mapping".into());
    };

    // VERSION first if present.
    if let Some(version) = m.get(Value::String("VERSION".into())) {
        let _ = writeln!(out, "VERSION {}", scalar_str(version));
    }
    if let Some(project) = m.get(Value::String("PROJECT".into())) {
        let _ = writeln!(out, "PROJECT {}", scalar_str(project));
    }
    out.push('\n');

    // Base recipe (commands before any target).
    if let Some(base) = m.get(Value::String("base_recipe".into())) {
        write_recipe(base, 0, &mut out);
        out.push('\n');
    }

    // Named targets.
    if let Some(Value::Mapping(targets)) = m.get(Value::String("targets".into())) {
        for (name, target) in targets {
            let target_name = scalar_str(name);
            let _ = writeln!(out, "{target_name}:");
            // target may be a recipe Mapping or a target-section
            // Mapping that has a `recipe` key. Handle both.
            if let Value::Mapping(tm) = target {
                if let Some(recipe) = tm.get(Value::String("recipe".into())) {
                    write_recipe(recipe, 4, &mut out);
                } else {
                    write_recipe(target, 4, &mut out);
                }
            }
            out.push('\n');
        }
    }
    Ok(out)
}

fn write_recipe(v: &Value, indent: usize, out: &mut String) {
    let Value::Mapping(r) = v else { return };
    for (verb, args) in r {
        let verb_s = scalar_str(verb);
        let args_s = scalar_str(args);
        for _ in 0..indent {
            out.push(' ');
        }
        if args_s.is_empty() {
            let _ = writeln!(out, "{verb_s}");
        } else {
            let _ = writeln!(out, "{verb_s} {args_s}");
        }
    }
}

fn scalar_str(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}
