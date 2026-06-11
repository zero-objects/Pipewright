//! Random walker: spec + seed → YAML value tree.
//!
//! The walker is a deterministic function of (rng, budget, spec).
//! Same seed + same budget = same output. This is what makes
//! proptest's shrinker useful — every failure can be reduced to a
//! minimal seed pair.

use crate::spec::{Field, Section, Spec, Type};
use rand::seq::SliceRandom;
use rand::Rng;
use serde_yaml::{Mapping, Value};

/// Generation budget. The walker decrements `depth_remaining` on
/// every recursion into a section/list/map; when it reaches 0
/// only required fields are emitted and list/map sizes saturate
/// at the minimum.
#[derive(Debug, Clone)]
pub struct Budget {
    /// Decrements on each recursion. Hard ceiling for nested
    /// structure depth — protects against blow-up on recursive
    /// section references (a platform whose `[step]` includes a
    /// nested `step` field would otherwise recurse forever).
    pub depth_remaining: u32,
    /// Probability an optional field is emitted (0.0 = required-
    /// only, 1.0 = always).
    pub optional_prob: f64,
    /// Cap on list / map child counts. Tuning knob for breadth
    /// vs. shrinkability — shorter lists shrink to minimal cases
    /// faster.
    pub max_collection: usize,
    /// Cap on number of fields per section (for sections with very
    /// many optionals like circleci's `[step]`). Walker still
    /// emits all required fields; only the optional pool is
    /// capped.
    pub max_optionals: usize,
}

impl Budget {
    pub fn shallow() -> Self {
        Self {
            depth_remaining: 4,
            optional_prob: 0.3,
            max_collection: 4,
            max_optionals: 6,
        }
    }
    pub fn deep() -> Self {
        Self {
            depth_remaining: 12,
            optional_prob: 0.7,
            max_collection: 12,
            max_optionals: 18,
        }
    }
    /// Aggressive — hits union arms, deeply nested optionals,
    /// large collections. Use for coverage/stress runs; chaos
    /// roundtrip cases at this budget take noticeably longer.
    pub fn substantial() -> Self {
        Self {
            depth_remaining: 20,
            optional_prob: 0.85,
            max_collection: 25,
            max_optionals: 40,
        }
    }
    fn dec(&self) -> Budget {
        let mut b = self.clone();
        b.depth_remaining = b.depth_remaining.saturating_sub(1);
        b
    }
}

/// Generate a value for the given root section.
pub fn gen_section(spec: &Spec, root: &str, rng: &mut impl Rng, budget: &Budget) -> Value {
    let Some(section) = spec.sections.get(root) else {
        return Value::Null;
    };
    gen_section_inner(spec, section, rng, budget)
}

fn gen_section_inner(spec: &Spec, section: &Section, rng: &mut impl Rng, budget: &Budget) -> Value {
    let mut m = Mapping::new();

    // Required fields first — these are mandatory by the platform
    // schema, so the walker emits them unconditionally.
    let required: Vec<&Field> = section.fields.values().filter(|f| f.required).collect();
    for f in &required {
        m.insert(
            Value::String(f.name.clone()),
            gen_type(spec, &f.ty, rng, &budget.dec(), f),
        );
    }

    // Optional fields: each an independent coin-flip (`optional_prob`).
    // No structural forcing, no empty-mapping safety — a section that
    // rolls nothing is a valid degenerate (`{}`), and structural
    // coverage comes from the schema marking those fields `required`,
    // not from the walker pretending optional fields are always present
    // (that forcing inflated coverage with configs the schema doesn't
    // actually guarantee).
    if budget.depth_remaining == 0 {
        return Value::Mapping(m);
    }
    let mut optional: Vec<&Field> = section.fields.values().filter(|f| !f.required).collect();
    optional.shuffle(rng);
    let cap = budget.max_optionals.min(optional.len());
    for f in optional.iter().take(cap) {
        if !rng.gen_bool(budget.optional_prob) {
            continue;
        }
        m.insert(
            Value::String(f.name.clone()),
            gen_type(spec, &f.ty, rng, &budget.dec(), f),
        );
    }
    Value::Mapping(m)
}

fn gen_type(spec: &Spec, ty: &Type, rng: &mut impl Rng, budget: &Budget, field: &Field) -> Value {
    match ty {
        Type::String | Type::Secret => Value::String(gen_word(rng)),
        Type::Number => Value::Number(rng.gen_range(0..1000).into()),
        Type::Integer => Value::Number(rng.gen_range(0..1000).into()),
        Type::Boolean => Value::Bool(rng.gen_bool(0.5)),
        Type::Null => Value::Null,
        Type::Any => Value::String(gen_word(rng)),
        Type::Object => Value::Mapping(Mapping::new()),
        Type::Enum => {
            if field.options.is_empty() {
                Value::String("enum".into())
            } else {
                let pick = field.options[rng.gen_range(0..field.options.len())].clone();
                Value::String(pick)
            }
        }
        Type::List(inner) => {
            let n = if budget.depth_remaining == 0 {
                1
            } else {
                rng.gen_range(1..=budget.max_collection.max(1))
            };
            let items = (0..n)
                .map(|_| gen_type(spec, inner, rng, &budget.dec(), field))
                .collect();
            Value::Sequence(items)
        }
        Type::Map(inner) => {
            let n = if budget.depth_remaining == 0 {
                1
            } else {
                rng.gen_range(1..=budget.max_collection.max(1))
            };
            let mut m = Mapping::new();
            let name_key = Value::String("name".to_string());
            for _ in 0..n {
                let key = Value::String(gen_word(rng));
                let mut val = gen_type(spec, inner, rng, &budget.dec(), field);
                // A map entry is keyed by its identity, so the key IS the
                // instance's name. An inner `name:` field would then collide
                // with the map-key name on the roundtrip (the seeder derives
                // name from the key AND from the field → two competing name
                // attrs, only one survives emit → not hub-stable). Real configs
                // never carry both; drop the redundant inner name so the chaos
                // corpus stays semantically valid.
                if let Value::Mapping(ref mut vm) = val {
                    vm.remove(&name_key);
                }
                m.insert(key, val);
            }
            Value::Mapping(m)
        }
        Type::Union(arms) => {
            // Pure random arm. (Once schemas are sharpened, unions no
            // longer carry useless Null/Object/Any arms to bias against.)
            let pick = &arms[rng.gen_range(0..arms.len())];
            gen_type(spec, pick, rng, budget, field)
        }
        Type::Section(name) => {
            let Some(section) = spec.sections.get(name) else {
                // Unknown bare name — degrade to string.
                return Value::String(gen_word(rng));
            };
            // At depth 0 gen_section_inner emits required fields only
            // (possibly `{}`) — a valid degenerate the pipeline handles.
            gen_section_inner(spec, section, rng, &budget.dec())
        }
    }
}

/// Word pool for scalar fillers. Sticking to a small alphabet
/// keeps generated documents readable when a shrinker reports
/// them; it also avoids tripping YAML's quoting rules.
fn gen_word(rng: &mut impl Rng) -> String {
    const WORDS: &[&str] = &[
        "alpha", "beta", "gamma", "delta", "epsilon", "build", "test", "deploy", "lint", "release",
        "main", "node", "linux", "darwin", "windows", "rust", "go", "python",
    ];
    WORDS[rng.gen_range(0..WORDS.len())].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn load_drone() -> Spec {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../catalog/drone.toml");
        crate::spec::load(&path).expect("load drone")
    }

    #[test]
    fn drone_pipeline_emits_required_keys() {
        let spec = load_drone();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let v = gen_section(&spec, "kind_pipeline", &mut rng, &Budget::shallow());
        let Value::Mapping(m) = v else {
            panic!("expected mapping")
        };
        // catalog/drone.toml flags name, steps and type as
        // required — those must appear on every seed.
        assert!(
            m.contains_key(Value::String("name".into())),
            "name missing: {m:?}"
        );
        assert!(
            m.contains_key(Value::String("steps".into())),
            "steps missing: {m:?}"
        );
        assert!(
            m.contains_key(Value::String("type".into())),
            "type missing: {m:?}"
        );
    }

    #[test]
    fn determinism() {
        let spec = load_drone();
        let mut rng1 = ChaCha8Rng::seed_from_u64(7);
        let mut rng2 = ChaCha8Rng::seed_from_u64(7);
        let v1 = gen_section(&spec, "kind_pipeline", &mut rng1, &Budget::shallow());
        let v2 = gen_section(&spec, "kind_pipeline", &mut rng2, &Budget::shallow());
        assert_eq!(v1, v2);
    }
}
