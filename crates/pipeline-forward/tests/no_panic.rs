//! Robustness: the public entry points must never PANIC on arbitrary input —
//! they return `Err`/`None` instead. A parser that panics on a crafted config
//! is a denial-of-service hole (and an ugly crash in the UI/FFI). This is the
//! property-test ("fuzz-lite") version of the manual garbage-input checks;
//! proptest shrinks any panicking input to a minimal reproducer.

use proptest::prelude::*;

const PLATFORMS: &[&str] = pipeline_forward::PLATFORMS;

proptest! {
    // Plenty of cases, but each is cheap (parse + lift, no Docker).
    #![proptest_config(ProptestConfig { cases: 1000, ..ProptestConfig::default() })]

    /// `detect` on any string: returns Some/None, never panics.
    #[test]
    fn detect_never_panics(s in ".*") {
        let _ = pipeline_forward::detect(&s);
        let _ = pipeline_forward::detect_with_path(&s, &s);
    }

    /// `forward` on any (platform, string): returns Ok/Err, never panics —
    /// for every platform.
    #[test]
    fn forward_never_panics(s in ".*", idx in 0usize..PLATFORMS.len()) {
        let _ = pipeline_forward::forward(PLATFORMS[idx], &s);
    }

    /// YAML-shaped noise (the realistic attack surface): keys, colons, dashes,
    /// indentation, anchors — fed to forward for every platform.
    #[test]
    fn forward_never_panics_on_yaml_noise(
        s in r"[a-z0-9_:\- \n\t\[\]{}&*#'\x22]{0,400}",
        idx in 0usize..PLATFORMS.len(),
    ) {
        let _ = pipeline_forward::forward(PLATFORMS[idx], &s);
    }
}

/// A handful of explicit nasty inputs as a fast, non-proptest smoke (these run
/// even when proptest is configured down).
#[test]
fn known_pathological_inputs_dont_panic() {
    let nasties: &[&str] = &[
        "",
        "\0\0\0",
        "\u{feff}", // BOM
        "key: [unclosed",
        "- - - - - -",
        ": : : :",
        &"a:\n".repeat(1000), // deep-ish nesting of keys
        &" ".repeat(10_000),  // whitespace flood
        "&a *a",              // anchor/alias fragment
        "\t\tweird: \ttabs",
    ];
    for &plat in PLATFORMS {
        for &n in nasties {
            let _ = pipeline_forward::forward(plat, n);
            let _ = pipeline_forward::detect(n);
        }
    }
}
