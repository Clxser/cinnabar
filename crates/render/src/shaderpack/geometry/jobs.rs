//! CPU planning of one section's expansion: which faces each packed record produces, in which
//! Iris geometry category, and the jobs the expansion kernels run to write them.
//!
//! A job is four words `[record, origin slot, first output face, flags]`. Cube jobs cover one
//! block of a greedy quad (its width and height steps in flag bits 0-15); every job reserves an
//! extra slot for a grass-side overlay ([`JOB_OVERLAY_FACE`]) and for the back of a two-sided
//! face ([`JOB_REVERSE_FACE`]) when its material may need them, and the kernel leaves unneeded
//! slots degenerate.

use std::ops::Range;

use assets::{Material, ModelQuad, ModelTemplate};
use meshing::{PackedLiquidQuad, PackedModelDrawRef, PackedModelRef, PackedQuad};

/// Job flag: the slot after the face holds a grass-side overlay face.
pub(crate) const JOB_OVERLAY_FACE: u32 = 1 << 16;
/// Job flag: the last slot holds the back of a two-sided face.
pub(crate) const JOB_REVERSE_FACE: u32 = 1 << 17;

/// Iris geometry categories, in the order a section's faces are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GeometryCategory {
    /// Opaque single-sided terrain and lava (`gbuffers_terrain`, Iris `gbuffers_terrain_solid`).
    Solid,
    /// Alpha-tested and two-sided terrain, block models and grass-side overlays
    /// (`gbuffers_terrain`, Iris `gbuffers_terrain_cutout`).
    Cutout,
    /// Water and translucent block models (`gbuffers_water`), drawn far to near by section.
    Translucent,
}

impl GeometryCategory {
    pub const ALL: [Self; 3] = [Self::Solid, Self::Cutout, Self::Translucent];

    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

/// Expansion kernel a job runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JobKind {
    Cube,
    Model,
    Liquid,
}

pub(crate) const JOB_KINDS: [JobKind; 3] = [JobKind::Cube, JobKind::Model, JobKind::Liquid];

/// Everything planning reads about one resident section. Record bases are the section's first
/// global record in each GPU stream, as its allocation placed them.
pub(crate) struct SectionGeometry<'a> {
    pub(crate) metadata_index: u32,
    /// Global index of the first cube quad.
    pub(crate) quad_base: u32,
    pub(crate) cube_quads: &'a [PackedQuad],
    /// Leading cube quads that are opaque and single-sided.
    pub(crate) solid_len: u32,
    pub(crate) model_refs: &'a [PackedModelRef],
    /// Global record of the first opaque model draw ref.
    pub(crate) model_draw_base: u32,
    pub(crate) model_draw_refs: &'a [PackedModelDrawRef],
    /// Global record of the first translucent model draw ref.
    pub(crate) transparent_model_draw_base: u32,
    pub(crate) transparent_model_draw_refs: &'a [PackedModelDrawRef],
    /// Global record of the first liquid record.
    pub(crate) liquid_base: u32,
    pub(crate) liquid_quads: &'a [PackedLiquidQuad],
}

/// The carrier tables planning resolves materials through.
pub(crate) struct MaterialTables<'a> {
    pub(crate) materials: &'a [Material],
    pub(crate) model_templates: &'a [ModelTemplate],
    pub(crate) model_quads: &'a [ModelQuad],
}

/// A planned section: its face count, per-category face ranges and kernel jobs, with output
/// faces relative to the section's first face until [`SectionPlan::place`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SectionPlan {
    pub(crate) faces: u32,
    pub(crate) categories: [Range<u32>; 3],
    pub(crate) jobs: [Vec<[u32; 4]>; 3],
}

impl SectionPlan {
    /// Moves every output face to start at arena face `first_face`.
    pub(crate) fn place(&mut self, first_face: u32) {
        for range in &mut self.categories {
            *range = range.start + first_face..range.end + first_face;
        }
        for jobs in &mut self.jobs {
            for job in jobs {
                job[2] += first_face;
            }
        }
    }
}

/// Slots a material may need beyond its face: an overlay when it or a positional variant
/// masks a grass-side overlay, a back face when any is two-sided.
fn material_flags(materials: &[Material], id: u32) -> u32 {
    let Some(base) = materials.get(id as usize) else {
        return 0;
    };
    let start = base.variation_start as usize;
    let variants = materials
        .get(start..start.saturating_add(base.variation_count as usize))
        .unwrap_or(&[]);
    let flags = variants
        .iter()
        .fold(base.flags, |flags, variant| flags | variant.flags);
    let mut job = 0;
    if flags & assets::MATERIAL_FLAG_OVERLAY_MASK != 0 {
        job |= JOB_OVERLAY_FACE;
    }
    if flags & assets::MATERIAL_FLAG_TWO_SIDED != 0 {
        job |= JOB_REVERSE_FACE;
    }
    job
}

/// Output slots of a job with `flags`.
pub(crate) const fn job_slots(flags: u32) -> u32 {
    1 + (flags & JOB_OVERLAY_FACE != 0) as u32 + (flags & JOB_REVERSE_FACE != 0) as u32
}

/// The template quad a model draw ref draws, if its addresses are in range.
fn model_material(
    tables: &MaterialTables<'_>,
    model_refs: &[PackedModelRef],
    draw: PackedModelDrawRef,
) -> Option<u32> {
    let [model_ref, quad_index] = draw.words();
    let template_id = model_refs.get(model_ref as usize)?.words()[1];
    let template = tables.model_templates.get(template_id as usize)?;
    (quad_index < template.quad_count).then_some(())?;
    let quad = tables
        .model_quads
        .get(template.quad_start.checked_add(quad_index)? as usize)?;
    Some(quad.material)
}

#[derive(Default)]
struct CategoryJobs {
    jobs: Vec<(JobKind, [u32; 4])>,
    faces: u32,
}

impl CategoryJobs {
    fn push(&mut self, kind: JobKind, record: u32, metadata_index: u32, flags: u32) {
        self.jobs
            .push((kind, [record, metadata_index, self.faces, flags]));
        self.faces += job_slots(flags);
    }
}

/// Plans one section. Overlay materials leave the solid run for the cutout category, because
/// their overlay face is alpha-tested.
pub(crate) fn plan_section(
    section: &SectionGeometry<'_>,
    tables: &MaterialTables<'_>,
) -> SectionPlan {
    let mut categories: [CategoryJobs; 3] = Default::default();
    let metadata = section.metadata_index;
    for (index, quad) in section.cube_quads.iter().enumerate() {
        let flags = material_flags(tables.materials, quad.material_id());
        let category = if (index as u32) < section.solid_len && flags & JOB_OVERLAY_FACE == 0 {
            GeometryCategory::Solid
        } else {
            GeometryCategory::Cutout
        };
        for height in 0..u32::from(quad.height()) {
            for width in 0..u32::from(quad.width()) {
                categories[category.index()].push(
                    JobKind::Cube,
                    section.quad_base + index as u32,
                    metadata,
                    flags | width | (height << 8),
                );
            }
        }
    }
    for (base, draws, category) in [
        (
            section.model_draw_base,
            section.model_draw_refs,
            GeometryCategory::Cutout,
        ),
        (
            section.transparent_model_draw_base,
            section.transparent_model_draw_refs,
            GeometryCategory::Translucent,
        ),
    ] {
        for (index, draw) in draws.iter().enumerate() {
            let flags = model_material(tables, section.model_refs, *draw)
                .map_or(0, |material| material_flags(tables.materials, material));
            categories[category.index()].push(JobKind::Model, base + index as u32, metadata, flags);
        }
    }
    for (index, liquid) in section.liquid_quads.iter().enumerate() {
        let category = if liquid.is_depth_writing() {
            GeometryCategory::Solid
        } else {
            GeometryCategory::Translucent
        };
        let flags = if liquid.is_two_sided() {
            JOB_REVERSE_FACE
        } else {
            0
        };
        categories[category.index()].push(
            JobKind::Liquid,
            section.liquid_base + index as u32,
            metadata,
            flags,
        );
    }

    let mut plan = SectionPlan::default();
    for (category, planned) in categories.into_iter().enumerate() {
        let first = plan.faces;
        plan.faces += planned.faces;
        plan.categories[category] = first..plan.faces;
        for (kind, mut job) in planned.jobs {
            job[2] += first;
            plan.jobs[kind as usize].push(job);
        }
    }
    plan
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;
