//! World-dependent model surfaces shared by terrain and block overlays.

use crate::{
    BlockFlags, MODEL_TEMPLATE_FLAG_FENCE_NETHER, MODEL_TEMPLATE_FLAG_FENCE_WOOD,
    MODEL_TEMPLATE_FLAG_GATE_AXIS_X, MODEL_TEMPLATE_FLAG_GATE_AXIS_Z, MODEL_TEMPLATE_FLAG_PANE,
    MODEL_TEMPLATE_FLAG_WALL, NetworkIdMode, RuntimeAssets, VisualKind,
    facts::connection::{SIDES, Side},
};

/// Maximum number of separately selected templates per block.
pub const MAX_MODEL_TEMPLATES: usize = 2;
/// A block may select its post and a separate set of connected rails.
pub type ModelTemplates = [Option<u32>; MAX_MODEL_TEMPLATES];

/// Neighbor facts needed to choose pane arms and fence rails.
#[derive(Clone, Copy)]
pub struct ConnectedModelNeighbor {
    pub template_flags: u32,
    pub occludes_full_face: bool,
    pub sequential_id: Option<u32>,
}

/// Selects pane surfaces or fence post/rails using neighbor flags and full-cube links.
/// Other template families return `None` without reading neighbors.
pub fn connected_templates(
    template: u32,
    flags: u32,
    mut neighbor: impl FnMut(Side) -> Option<ConnectedModelNeighbor>,
) -> Option<ModelTemplates> {
    let pane = flags & MODEL_TEMPLATE_FLAG_PANE != 0;
    let fence = flags & (MODEL_TEMPLATE_FLAG_FENCE_WOOD | MODEL_TEMPLATE_FLAG_FENCE_NETHER);
    if !pane && fence == 0 {
        return None;
    }
    let mut mask = 0;
    for side in SIDES {
        let Some(neighbor) = neighbor(side) else {
            continue;
        };
        let flags = neighbor.template_flags;
        let links = if pane {
            flags & (MODEL_TEMPLATE_FLAG_PANE | MODEL_TEMPLATE_FLAG_WALL) != 0
        } else {
            flags & fence != 0
                || flags
                    & if side.offset[0] == 0 {
                        MODEL_TEMPLATE_FLAG_GATE_AXIS_X
                    } else {
                        MODEL_TEMPLATE_FLAG_GATE_AXIS_Z
                    }
                    != 0
        } || neighbor.occludes_full_face
            || neighbor.sequential_id.is_some_and(|id| {
                crate::facts::connection::pinned_links_as_unit_cube(NetworkIdMode::Sequential, id)
            });
        if links {
            // Template masks advance clockwise from north through east, south and west.
            let bit = match side.face {
                2 => 0,
                5 => 1,
                3 => 2,
                4 => 3,
                _ => unreachable!("horizontal connection side"),
            };
            mask |= 1 << bit;
        }
    }
    Some(if pane {
        [Some(template + mask), None]
    } else {
        [Some(template), (mask != 0).then_some(template + 1 + mask)]
    })
}

impl RuntimeAssets {
    /// Resolves connected surfaces and paired door poses against current primary neighbors.
    /// Walls and other fixed models retain their state-selected templates.
    pub fn model_templates_at(
        &self,
        mode: NetworkIdMode,
        runtime_id: u32,
        block: [i32; 3],
        mut runtime_id_at: impl FnMut([i32; 3]) -> Option<u32>,
    ) -> ModelTemplates {
        let visual = self.resolve(mode, runtime_id);
        let Some(template) = visual.model_template() else {
            return [None; 2];
        };
        let flags = self
            .model_templates()
            .get(template as usize)
            .map_or(0, |part| part.flags);
        if let Some(templates) = connected_templates(template, flags, |side| {
            let position = [
                block[0].checked_add(side.offset[0])?,
                block[1],
                block[2].checked_add(side.offset[2])?,
            ];
            let id = runtime_id_at(position)?;
            let other = self.resolve(mode, id);
            let flags = (other.kind() == VisualKind::Model)
                .then(|| {
                    other
                        .model_template()
                        .and_then(|index| self.model_templates().get(index as usize))
                })
                .flatten()
                .map_or(0, |part| part.flags);
            let sequential = match mode {
                NetworkIdMode::Sequential => Some(id),
                NetworkIdMode::Hashed => self.sequential_id_for_hash(id),
            };
            Some(ConnectedModelNeighbor {
                template_flags: flags,
                occludes_full_face: other.flags().contains(BlockFlags::OCCLUDES_FULL_FACE),
                sequential_id: sequential,
            })
        }) {
            return templates;
        }
        [
            crate::door::resolve_template(self, template, visual.variant(), |upper| {
                let y = block[1].checked_add(if upper { -1 } else { 1 })?;
                let other = self.resolve(mode, runtime_id_at([block[0], y, block[2]])?);
                if other.kind() != VisualKind::Model {
                    return None;
                }
                Some((other.model_template()?, other.variant()))
            }),
            None,
        ]
    }
}
