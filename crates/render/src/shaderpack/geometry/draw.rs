//! Per-view draw lists over resident expanded sections, retained while what they were built
//! from holds, and recording them into a pass.

use std::ops::Range;

use bevy::math::DVec3;
use bevy::render::render_phase::TrackedRenderPass;
use bevy::render::render_resource::{Buffer, BufferDescriptor, BufferUsages, IndexFormat};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use wgpu::util::DrawIndexedIndirectArgs;

use super::jobs::{FACING_GROUPS, FACING_REST, GeometryCategory};
use super::residency::{ResidentFaces, section_distance};
use super::vertex::{CONSTANT_VERTEX_BUFFER, GEOMETRY_VERTEX_BUFFER};

/// Indices of one quad face, counter-clockwise like its corners.
pub(crate) const FACE_INDICES: [u32; 6] = [0, 1, 2, 0, 2, 3];
/// Edge of a section, in blocks.
const SECTION_SIZE: f64 = 16.0;

/// Consecutive commands of one category that read the same vertex page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DrawRun {
    pub(crate) page: usize,
    pub(crate) commands: Range<u32>,
}

/// One view's draw commands: every category's commands back to back, split into page runs.
#[derive(Default)]
pub(crate) struct ViewDraws {
    pub(crate) commands: Vec<DrawIndexedIndirectArgs>,
    pub(crate) runs: [Vec<DrawRun>; 3],
    /// The commands as indirect arguments, when the device draws terrain through
    /// multi-draw-indirect; kept across rebuilds and grown when they outgrow it.
    indirect: Option<Buffer>,
    /// Fingerprint of the inputs the lists were built from.
    key: Option<u64>,
}

/// The facing groups of the section at block `origin` whose faces `eye` may see from the
/// front: an axis-facing face lies on a plane within the section's extent and faces the eye
/// only from that plane's front side, as Sodium's block face culling decides.
fn facing_visible(origin: [i32; 3], eye: DVec3) -> [bool; FACING_GROUPS] {
    let min = DVec3::new(
        f64::from(origin[0]),
        f64::from(origin[1]),
        f64::from(origin[2]),
    );
    let max = min + DVec3::splat(SECTION_SIZE);
    let mut visible = [true; FACING_GROUPS];
    for axis in 0..3 {
        // `meshing::Face` order: the negative, then the positive direction of each axis.
        visible[axis * 2] = eye[axis] <= max[axis];
        visible[axis * 2 + 1] = eye[axis] >= min[axis];
    }
    visible[FACING_REST] = true;
    visible
}

impl ViewDraws {
    /// Lists `sections` for one view: solid and cutout in page order, translucent far to near
    /// from `eye` as Iris draws translucent terrain by section. With `facing`, the camera
    /// position, solid and cutout faces it can only see from behind are left out.
    pub(crate) fn build<'a>(
        sections: impl Iterator<Item = &'a ResidentFaces>,
        eye: Option<DVec3>,
        epoch: u64,
        facing: Option<DVec3>,
    ) -> Self {
        let sections: Vec<_> = sections.filter(|section| section.epoch == epoch).collect();
        let mut draws = Self::default();
        for category in GeometryCategory::ALL {
            let mut ordered: Vec<_> = sections
                .iter()
                .filter(|section| !section.categories[category.index()].is_empty())
                .collect();
            match (category, eye) {
                (GeometryCategory::Translucent, Some(eye)) => ordered.sort_by(|a, b| {
                    section_distance(b.origin, eye).total_cmp(&section_distance(a.origin, eye))
                }),
                _ => ordered.sort_by_key(|section| (section.page, section.faces.start)),
            }
            let culls_facing = category != GeometryCategory::Translucent;
            for section in ordered {
                let groups = &section.facing[category.index()];
                let visible = facing
                    .filter(|_| culls_facing)
                    .map_or([true; FACING_GROUPS], |eye| {
                        facing_visible(section.origin, eye)
                    });
                // Visible groups that follow one another merge into one command.
                let mut pending: Option<Range<u32>> = None;
                for (group, faces) in groups.iter().enumerate() {
                    if !visible[group] || faces.is_empty() {
                        continue;
                    }
                    match &mut pending {
                        Some(open) if open.end == faces.start => open.end = faces.end,
                        _ => {
                            if let Some(done) = pending.replace(faces.clone()) {
                                draws.push(category, section.page, done);
                            }
                        }
                    }
                }
                if let Some(done) = pending {
                    draws.push(category, section.page, done);
                }
            }
        }
        draws
    }

    fn push(&mut self, category: GeometryCategory, page: usize, faces: Range<u32>) {
        let command = self.commands.len() as u32;
        self.commands.push(DrawIndexedIndirectArgs {
            index_count: (faces.end - faces.start) * FACE_INDICES.len() as u32,
            instance_count: 1,
            first_index: 0,
            base_vertex: (faces.start * 4) as i32,
            first_instance: 0,
        });
        let runs = &mut self.runs[category.index()];
        match runs.last_mut() {
            Some(run) if run.page == page && run.commands.end == command => {
                run.commands.end += 1;
            }
            _ => runs.push(DrawRun {
                page,
                commands: command..command + 1,
            }),
        }
    }

    /// Whether the lists were built from inputs with fingerprint `key`.
    pub(crate) fn is_current(&self, key: u64) -> bool {
        self.key == Some(key)
    }

    /// Takes `built`'s lists as the ones for inputs `key`, writing them into the retained
    /// indirect buffer (grown to fit) when the device draws through multi-draw-indirect.
    pub(crate) fn replace(
        &mut self,
        built: ViewDraws,
        key: u64,
        upload: Option<(&RenderDevice, &RenderQueue)>,
    ) {
        self.commands = built.commands;
        self.runs = built.runs;
        self.key = Some(key);
        let Some((device, queue)) = upload else {
            self.indirect = None;
            return;
        };
        if self.commands.is_empty() {
            return;
        }
        let bytes: Vec<u8> = self
            .commands
            .iter()
            .flat_map(|command| command.as_bytes())
            .copied()
            .collect();
        let size = bytes.len() as u64;
        if self
            .indirect
            .as_ref()
            .is_none_or(|buffer| buffer.size() < size)
        {
            self.indirect = Some(device.create_buffer(&BufferDescriptor {
                label: Some("shader-pack terrain draws"),
                size: size.next_power_of_two(),
                usage: BufferUsages::INDIRECT | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if let Some(buffer) = &self.indirect {
            queue.write_buffer(buffer, 0, &bytes);
        }
    }

    /// Largest command in faces, which the shared index buffer must cover.
    pub(crate) fn max_faces(&self) -> u32 {
        self.commands
            .iter()
            .map(|command| command.index_count / FACE_INDICES.len() as u32)
            .max()
            .unwrap_or(0)
    }

    /// Records `category` into `pass`, which must already have the program's pipeline and bind
    /// groups set. Returns the draws issued.
    pub(crate) fn record<'w>(
        &'w self,
        pass: &mut TrackedRenderPass<'w>,
        category: GeometryCategory,
        pages: &'w [Buffer],
        index_buffer: &'w Buffer,
        constants: &'w Buffer,
    ) -> u32 {
        let runs = &self.runs[category.index()];
        if runs.is_empty() {
            return 0;
        }
        pass.set_index_buffer(index_buffer.slice(..), IndexFormat::Uint32);
        pass.set_vertex_buffer(CONSTANT_VERTEX_BUFFER as usize, constants.slice(..));
        let stride = std::mem::size_of::<DrawIndexedIndirectArgs>() as u64;
        for run in runs {
            pass.set_vertex_buffer(GEOMETRY_VERTEX_BUFFER as usize, pages[run.page].slice(..));
            match &self.indirect {
                Some(indirect) => pass.multi_draw_indexed_indirect(
                    indirect,
                    u64::from(run.commands.start) * stride,
                    run.commands.end - run.commands.start,
                ),
                None => {
                    for command in
                        &self.commands[run.commands.start as usize..run.commands.end as usize]
                    {
                        pass.draw_indexed(0..command.index_count, command.base_vertex, 0..1);
                    }
                }
            }
        }
        runs.iter()
            .map(|run| run.commands.end - run.commands.start)
            .sum()
    }
}

/// Index buffer words for `faces` consecutive faces.
pub(crate) fn face_indices(faces: u32) -> Vec<u32> {
    (0..faces)
        .flat_map(|face| FACE_INDICES.map(|index| face * 4 + index))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rest(range: Range<u32>) -> [Range<u32>; FACING_GROUPS] {
        let mut groups: [Range<u32>; FACING_GROUPS] =
            std::array::from_fn(|_| range.start..range.start);
        groups[FACING_REST] = range;
        groups
    }

    fn section(
        page: usize,
        faces: Range<u32>,
        translucent: Range<u32>,
        origin: [i32; 3],
    ) -> ResidentFaces {
        let categories = [
            faces.start..translucent.start,
            translucent.start..translucent.start,
            translucent,
        ];
        ResidentFaces {
            page,
            faces: faces.clone(),
            facing: categories.clone().map(rest),
            categories,
            origin,
            epoch: 1,
        }
    }

    #[test]
    fn translucent_sections_draw_far_to_near_and_stale_epochs_are_skipped() {
        let near = section(0, 0..4, 2..4, [0, 0, 0]);
        let far = section(0, 4..8, 6..8, [64, 0, 0]);
        let mut stale = section(1, 0..2, 2..2, [0, 16, 0]);
        stale.epoch = 0;
        let draws = ViewDraws::build(
            [&near, &far, &stale].into_iter(),
            Some(DVec3::ZERO),
            1,
            None,
        );
        let translucent: Vec<_> = draws.runs[GeometryCategory::Translucent.index()]
            .iter()
            .flat_map(|run| run.commands.clone())
            .map(|command| draws.commands[command as usize].base_vertex)
            .collect();
        assert_eq!(translucent, [6 * 4, 2 * 4], "far section first");
        let solid = &draws.runs[GeometryCategory::Solid.index()];
        assert_eq!(solid.len(), 1, "both solid ranges share page 0 in one run");
        assert_eq!(solid[0].commands.len(), 2);
        assert_eq!(draws.max_faces(), 2);
    }

    #[test]
    fn faces_seen_only_from_behind_are_left_out_and_visible_groups_merge() {
        // Solid faces 0..12, two per facing group in `meshing::Face` order, none in the rest.
        let mut section = section(0, 0..12, 12..12, [0, 0, 0]);
        section.facing[0] = std::array::from_fn(|group| {
            let start = (group as u32 * 2).min(12);
            start..(start + 2).min(12)
        });
        // Above and beyond +X and +Z of the section: -X, -Y and -Z faces face away.
        let eye = DVec3::new(40.0, 40.0, 40.0);
        let draws = ViewDraws::build([&section].into_iter(), Some(eye), 1, Some(eye));
        let solid: Vec<_> = draws
            .commands
            .iter()
            .map(|c| (c.base_vertex / 4, c.index_count / 6))
            .collect();
        assert_eq!(
            solid,
            [(2, 2), (6, 2), (10, 2)],
            "+X, +Y and +Z groups only"
        );
        // Inside the section every group may face the eye: one merged command.
        let inside = DVec3::splat(8.0);
        let draws = ViewDraws::build([&section].into_iter(), Some(inside), 1, Some(inside));
        assert_eq!(draws.commands.len(), 1);
        assert_eq!(draws.max_faces(), 12);
    }

    #[test]
    fn face_indices_form_two_counter_clockwise_triangles_per_face() {
        assert_eq!(face_indices(2), [0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]);
    }
}
