//! Per-view draw lists over resident expanded sections, and recording them into a pass.

use std::ops::Range;

use bevy::math::{DVec3, IVec3};
use bevy::render::render_phase::TrackedRenderPass;
use bevy::render::render_resource::{Buffer, BufferInitDescriptor, BufferUsages, IndexFormat};
use bevy::render::renderer::RenderDevice;
use wgpu::util::DrawIndexedIndirectArgs;

use super::jobs::GeometryCategory;
use super::residency::ResidentFaces;
use super::vertex::{CONSTANT_VERTEX_BUFFER, GEOMETRY_VERTEX_BUFFER};

/// Indices of one quad face, counter-clockwise like its corners.
pub(crate) const FACE_INDICES: [u32; 6] = [0, 1, 2, 0, 2, 3];

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
    /// multi-draw-indirect.
    pub(crate) indirect: Option<Buffer>,
}

/// Distance order key of a section from `eye`: its centre's squared distance.
fn section_distance(origin: [i32; 3], eye: DVec3) -> f64 {
    let centre = IVec3::from_array(origin).as_dvec3() + DVec3::splat(8.0);
    centre.distance_squared(eye)
}

impl ViewDraws {
    /// Lists `sections` for one view: solid and cutout in page order, translucent far to near
    /// from `eye` as Iris draws translucent terrain by section.
    pub(crate) fn build<'a>(
        sections: impl Iterator<Item = &'a ResidentFaces>,
        eye: Option<DVec3>,
        epoch: u64,
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
            let runs = &mut draws.runs[category.index()];
            for section in ordered {
                let faces = &section.categories[category.index()];
                let command = draws.commands.len() as u32;
                draws.commands.push(DrawIndexedIndirectArgs {
                    index_count: (faces.end - faces.start) * FACE_INDICES.len() as u32,
                    instance_count: 1,
                    first_index: 0,
                    base_vertex: (faces.start * 4) as i32,
                    first_instance: 0,
                });
                match runs.last_mut() {
                    Some(run) if run.page == section.page && run.commands.end == command => {
                        run.commands.end += 1;
                    }
                    _ => runs.push(DrawRun {
                        page: section.page,
                        commands: command..command + 1,
                    }),
                }
            }
        }
        draws
    }

    /// Uploads the commands for multi-draw-indirect submission.
    pub(crate) fn upload_indirect(&mut self, device: &RenderDevice) {
        self.indirect = (!self.commands.is_empty()).then(|| {
            let bytes: Vec<u8> = self
                .commands
                .iter()
                .flat_map(|command| command.as_bytes().iter().copied())
                .collect();
            device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("shader-pack terrain draws"),
                contents: &bytes,
                usage: BufferUsages::INDIRECT,
            })
        });
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

    fn section(
        page: usize,
        faces: Range<u32>,
        translucent: Range<u32>,
        origin: [i32; 3],
    ) -> ResidentFaces {
        ResidentFaces {
            page,
            faces: faces.clone(),
            categories: [
                faces.start..translucent.start,
                translucent.start..translucent.start,
                translucent,
            ],
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
        let draws = ViewDraws::build([&near, &far, &stale].into_iter(), Some(DVec3::ZERO), 1);
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
    fn face_indices_form_two_counter_clockwise_triangles_per_face() {
        assert_eq!(face_indices(2), [0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]);
    }
}
