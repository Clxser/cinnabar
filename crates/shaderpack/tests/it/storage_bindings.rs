//! Storage binding boundaries preserve resource identity and program isolation.

use crate::{program, reflection};
use shaderpack::{PackSource, PackTranslation, Phase, ShaderPack, StageMask, TranslateConfig};

const VERTEX: &str = "#version 450\nvoid main() { gl_Position = vec4(0.0); }";
const FRAGMENT: &str = "#version 450\nout vec4 color; void main() { color = vec4(1.0); }";

/// Translates a storage program alongside an independent, healthy render program.
fn translate(vertex: &str, fragment: &str) -> PackTranslation {
    let translated = ShaderPack::new(PackSource::from_files([
        ("composite.vsh", vertex),
        ("composite.fsh", fragment),
        ("final.vsh", VERTEX),
        ("final.fsh", FRAGMENT),
    ]))
    .translate(&TranslateConfig::default());
    reflection(program(&translated, "final"));
    translated
}

/// Requires each named storage resource to retain its binding without aliases.
fn bindings(translated: &PackTranslation, expected: &[(&str, u32)]) {
    let reflected = reflection(program(translated, "composite"));
    let actual: Vec<_> = reflected
        .storage_buffers
        .iter()
        .map(|buffer| (buffer.name.as_str(), buffer.binding))
        .collect();
    for binding in expected {
        assert!(
            actual.contains(binding),
            "missing {binding:?} in {actual:?}"
        );
    }
    assert_eq!(actual.len(), expected.len());
}

#[test]
fn storage_bindings_allow_an_explicit_block_after_the_upper_boundary() {
    let translated = translate(
        VERTEX,
        "#version 450
        layout(binding=4294967295u) readonly buffer High { vec4 value; } high;
        layout(binding=7) readonly buffer Next { vec4 value; } next;
        out vec4 color; void main() { color = high.value + next.value; }",
    );
    bindings(&translated, &[("high", u32::MAX), ("next", 7)]);
}

#[test]
fn storage_bindings_find_a_gap_after_the_upper_boundary() {
    let translated = translate(
        VERTEX,
        "#version 450
        layout(binding=0) readonly buffer Low { vec4 value; } low;
        layout(binding=4294967295u) readonly buffer High { vec4 value; } high;
        readonly buffer Next { vec4 value; } next;
        out vec4 color; void main() { color = high.value + low.value + next.value; }",
    );
    bindings(&translated, &[("high", u32::MAX), ("low", 0), ("next", 1)]);
}

#[test]
fn storage_bindings_reuse_the_upper_boundary_across_stages() {
    for qualifier in ["layout(binding=4294967295u)", ""] {
        let fragment = format!(
            "#version 450
            {qualifier} readonly buffer High {{ vec4 value; }} high;
            out vec4 color; void main() {{ color = high.value; }}"
        );
        let translated = translate(
            "#version 450
            layout(binding=4294967295u) readonly buffer High { vec4 value; } high;
            void main() { gl_Position = high.value; }",
            &fragment,
        );
        bindings(&translated, &[("high", u32::MAX)]);
        assert_eq!(
            reflection(program(&translated, "composite")).storage_buffers[0].stages,
            StageMask {
                vertex: true,
                fragment: true,
                compute: false,
            }
        );
    }
}

#[test]
fn storage_bindings_reject_distinct_blocks_claiming_the_same_slot() {
    for (vertex, fragment, line) in [
        (
            VERTEX,
            "#version 450
            layout(binding=3) readonly buffer First { vec4 value; } first;
            layout(binding=3) readonly buffer Second { vec4 value; } second;
            out vec4 color; void main() { color = first.value + second.value; }",
            3,
        ),
        (
            "#version 450
            layout(binding=3) readonly buffer First { vec4 value; } first;
            void main() { gl_Position = first.value; }",
            "#version 450
            layout(binding=3) readonly buffer Second { vec4 value; } second;
            out vec4 color; void main() { color = second.value; }",
            2,
        ),
    ] {
        let translated = translate(vertex, fragment);
        let failed = program(&translated, "composite");
        assert!(!failed.is_ok());
        assert!(failed.all_diagnostics().any(|d| {
            d.phase == Phase::Transform
                && d.location.as_ref().is_some_and(|location| {
                    location.file == "composite.fsh" && location.line == line
                })
        }));
    }
}

#[test]
fn storage_bindings_reject_cross_stage_binding_disagreement() {
    let translated = translate(
        "#version 450
        layout(binding=2) readonly buffer Shared { vec4 value; } sharedData;
        void main() { gl_Position = sharedData.value; }",
        "#version 450
        layout(binding=4) readonly buffer Shared { vec4 value; } sharedData;
        out vec4 color; void main() { color = sharedData.value; }",
    );
    let failed = program(&translated, "composite");
    assert!(!failed.is_ok());
    assert!(failed.all_diagnostics().any(|d| {
        d.phase == Phase::Transform
            && d.location
                .as_ref()
                .is_some_and(|location| location.file == "composite.fsh" && location.line == 2)
    }));
}
