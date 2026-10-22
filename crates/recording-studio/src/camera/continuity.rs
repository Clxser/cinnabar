//! Optional tangent alignment gives arc-length playback a shared direction through ordinary joins.

use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathContinuity {
    #[default]
    Authored,
    Smooth,
}

impl PathContinuity {
    /// Original control geometry is the compatibility default and is omitted from project JSON.
    pub const fn is_authored(&self) -> bool {
        matches!(self, Self::Authored)
    }

    /// Labels describe an explicit geometric edit, distinct from the rail's travel timing.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Authored => "Authored tangents",
            Self::Smooth => "Aligned tangents",
        }
    }
}

/// Both sides of an interior key use one direction while retaining their independent handle lengths.
pub(super) fn controls(keys: &[CameraKey], index: usize) -> [[f64; 3]; 2] {
    let start = &keys[index];
    let end = &keys[index + 1];
    let length =
        DVec3::from_array(end.pose.position).distance(DVec3::from_array(start.pose.position));
    [
        (DVec3::from_array(start.pose.position)
            + direction(keys, index)
                * start
                    .handles
                    .and_then(|handles| handles.outgoing)
                    .map_or(length / 3.0, |handle| DVec3::from_array(handle).length()))
        .to_array(),
        (DVec3::from_array(end.pose.position)
            - direction(keys, index + 1)
                * end
                    .handles
                    .and_then(|handles| handles.incoming)
                    .map_or(length / 3.0, |handle| DVec3::from_array(handle).length()))
        .to_array(),
    ]
}

/// Explicit handles choose the direction first; opposing handles and true reversals retain a cusp.
fn direction(keys: &[CameraKey], index: usize) -> DVec3 {
    let key = &keys[index];
    let has_authored = key.handles.is_some_and(|handles| {
        handles
            .incoming
            .into_iter()
            .chain(handles.outgoing)
            .any(|handle| DVec3::from_array(handle).length_squared() > 0.0)
    });
    let authored = key
        .handles
        .map(|handles| {
            unit(
                handles
                    .outgoing
                    .map(DVec3::from_array)
                    .unwrap_or(DVec3::ZERO),
            ) - unit(
                handles
                    .incoming
                    .map(DVec3::from_array)
                    .unwrap_or(DVec3::ZERO),
            )
        })
        .unwrap_or(DVec3::ZERO);
    if has_authored {
        return unit(authored);
    }
    let position = DVec3::from_array(key.pose.position);
    let incoming = index
        .checked_sub(1)
        .map(|before| position - DVec3::from_array(keys[before].pose.position))
        .unwrap_or(DVec3::ZERO);
    let outgoing = keys
        .get(index + 1)
        .map(|after| DVec3::from_array(after.pose.position) - position)
        .unwrap_or(DVec3::ZERO);
    unit(unit(incoming) + unit(outgoing))
}

/// Stationary controls remain exactly zero instead of manufacturing a direction or NaN.
fn unit(value: DVec3) -> DVec3 {
    value.try_normalize().unwrap_or(DVec3::ZERO)
}
