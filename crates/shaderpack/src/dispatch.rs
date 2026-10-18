//! Compute dispatch settings from active source directives.

use crate::{preprocess::ConstDirective, reflection::ComputeDispatch};

/// Applies recognized literal constructors in source order, retaining the last valid value.
pub(crate) fn reflect(workgroup_size: [u32; 3], directives: &[ConstDirective]) -> ComputeDispatch {
    let mut dispatch = ComputeDispatch {
        workgroup_size,
        work_groups: None,
        work_groups_render: None,
    };
    for directive in directives {
        match (directive.ty.as_str(), directive.name.as_str()) {
            ("ivec3", "workGroups") => {
                if let Some(value) =
                    components(&directive.value, "ivec3", |n| n.parse::<u32>().ok())
                {
                    dispatch.work_groups = Some(value);
                }
            }
            ("vec2", "workGroupsRender") => {
                if let Some(value) = components(&directive.value, "vec2", |n| {
                    n.trim_end_matches(['f', 'F', 'd', 'D'])
                        .parse::<f32>()
                        .ok()
                        .filter(|v| v.is_finite())
                }) {
                    dispatch.work_groups_render = Some(value);
                }
            }
            _ => {}
        }
    }
    dispatch
}

/// Reads exactly N scalar literals from a named vector constructor without evaluating GLSL.
fn components<T, const N: usize>(
    value: &str,
    constructor: &str,
    parse: impl Fn(&str) -> Option<T>,
) -> Option<[T; N]> {
    let arguments = value
        .strip_prefix(constructor)?
        .trim()
        .strip_prefix('(')?
        .strip_suffix(')')?;
    let values: Option<Vec<T>> = arguments
        .split(',')
        .map(|value| parse(value.trim()))
        .collect();
    values?.try_into().ok()
}
