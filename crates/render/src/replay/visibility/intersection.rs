//! Editor-only per-bone boxes approximate occupied rig volume, not pixel alpha or exact triangles.
//! One immutable geometry snapshot and one posed spatial index are retained by each replay renderer.

use std::{collections::BTreeMap, sync::Arc};

use glam::{Vec3, Vec4};
use recording_studio::camera::CameraPose;
use render_model::{ActorRigGeometrySpan, ActorRigRenderFrame, ActorRigVertexSegments};

use super::{
    DrawKey,
    volume::{Bounds, NearVolume},
};

#[derive(Clone, Copy)]
struct RestBone {
    index: usize,
    bounds: Bounds,
}

#[derive(Default)]
struct RestMesh {
    bones: Arc<[RestBone]>,
    bridges: bool,
}

#[derive(Default)]
struct Geometry {
    source: Option<(ActorRigVertexSegments, Arc<[ActorRigGeometrySpan]>)>,
    bounds: BTreeMap<u32, Arc<RestMesh>>,
}

struct Part {
    key: DrawKey,
    bounds: Bounds,
    bones: Vec<[Vec3; 8]>,
}

#[derive(Clone, Copy)]
enum Children {
    Leaf(usize),
    Branch([usize; 2]),
}

struct Node {
    bounds: Bounds,
    children: Children,
}

#[derive(Default)]
pub(in crate::replay) struct IntersectionCache {
    geometry: Geometry,
    source: Option<ActorRigRenderFrame>,
    parts: Vec<Part>,
    nodes: Vec<Node>,
    root: Option<usize>,
    #[cfg(test)]
    pub(super) geometry_vertices: usize,
    #[cfg(test)]
    pub(super) pose_builds: usize,
    #[cfg(test)]
    pub(super) tested_parts: std::cell::Cell<usize>,
}

impl IntersectionCache {
    /// Recorded-camera policy needs no custom bounds; dropping the cache releases old take roots.
    pub(in crate::replay) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Camera-only updates query a balanced spatial index instead of scanning actors or vertices.
    pub(in crate::replay) fn mask(
        &mut self,
        rig: &ActorRigRenderFrame,
        pose: CameraPose,
        size: [u32; 2],
    ) -> Vec<DrawKey> {
        let Some(volume) = NearVolume::new(pose, size) else {
            return Vec::new();
        };
        if self
            .source
            .as_ref()
            .is_none_or(|old| !crate::replay::actor_frame::same_rig(old, rig))
        {
            self.prepare(rig);
        }
        let mut mask = Vec::new();
        if let Some(root) = self.root {
            self.query(root, &volume, &mut mask);
        }
        mask.sort_unstable();
        mask.dedup();
        mask
    }

    /// Every immutable pose input participates; source generation numbers alone cannot prove reuse.
    fn prepare(&mut self, rig: &ActorRigRenderFrame) {
        self.parts.clear();
        self.nodes.clear();
        self.root = None;
        self.source = None;
        let same_geometry = self
            .geometry
            .source
            .as_ref()
            .is_some_and(|(vertices, spans)| {
                vertices.epoch == rig.geometry_vertices.epoch
                    && vertices.len() == rig.geometry_vertices.len()
                    && Arc::ptr_eq(&vertices.segments, &rig.geometry_vertices.segments)
                    && Arc::ptr_eq(&vertices.offsets, &rig.geometry_vertices.offsets)
                    && Arc::ptr_eq(spans, &rig.geometry_spans)
            });
        if !same_geometry {
            self.geometry.bounds.clear();
            self.geometry.source = Some((
                rig.geometry_vertices.clone(),
                Arc::clone(&rig.geometry_spans),
            ));
        }
        #[cfg(test)]
        {
            self.pose_builds += 1;
        }
        for (index, instance) in rig.instances.iter().enumerate() {
            let Some(manifest) = rig.manifest.get(index) else {
                continue;
            };
            if !instance.partial_tick.is_finite() {
                continue;
            }
            if let std::collections::btree_map::Entry::Vacant(entry) =
                self.geometry.bounds.entry(instance.geometry_id)
            {
                let bounds = rig
                    .geometry_spans
                    .get(instance.geometry_id as usize)
                    .and_then(|&span| rig.geometry_vertices.span(span))
                    .and_then(|vertices| {
                        #[cfg(test)]
                        {
                            self.geometry_vertices += vertices.len();
                        }
                        rest_bounds(vertices)
                    })
                    .unwrap_or_default();
                entry.insert(bounds);
            }
            let rest = &self.geometry.bounds[&instance.geometry_id];
            let mut bones = Vec::with_capacity(rest.bones.len() + usize::from(rest.bridges));
            for bone in rest.bones.iter() {
                if bone.index >= manifest.bone_count as usize {
                    continue;
                }
                let (Some(previous), Some(current)) = (
                    (instance.previous_bone_base as usize)
                        .checked_add(bone.index)
                        .and_then(|index| rig.previous_bones.get(index)),
                    (instance.current_bone_base as usize)
                        .checked_add(bone.index)
                        .and_then(|index| rig.current_bones.get(index)),
                ) else {
                    continue;
                };
                let corners: [Vec3; 8] = std::array::from_fn(|corner| {
                    let local = Vec3::from_array(std::array::from_fn(|axis| {
                        if corner & (1 << axis) == 0 {
                            bone.bounds.min[axis] as f32
                        } else {
                            bone.bounds.max[axis] as f32
                        }
                    }));
                    let posed = transform(*previous, local) * (1.0 - instance.partial_tick)
                        + transform(*current, local) * instance.partial_tick;
                    transform(instance.world_from_actor, posed)
                });
                if Bounds::points(&corners.map(|corner| corner.as_dvec3())).is_some() {
                    bones.push(corners);
                }
            }
            let bounds = bones
                .iter()
                .filter_map(|corners| Bounds::points(&corners.map(|corner| corner.as_dvec3())))
                .reduce(Bounds::union);
            if let Some(bounds) = bounds {
                // Cross-bone triangles can bridge two boxes; their complete posed envelope remains
                // conservative until triangle-level picking is needed for these uncommon meshes.
                if rest.bridges {
                    bones.push(bounds.corners().map(|corner| corner.as_vec3()));
                }
                self.parts.push(Part {
                    key: DrawKey::from(&manifest.identity),
                    bones,
                    bounds,
                });
            }
        }
        if !self.parts.is_empty() {
            let mut indices = (0..self.parts.len()).collect::<Vec<_>>();
            self.root = Some(build(&self.parts, &mut indices, &mut self.nodes));
        }
        self.source = Some(rig.clone());
    }

    /// A branch is visited only when its world bounds overlap the small eye/near-plane pyramid.
    fn query(&self, node: usize, volume: &NearVolume, mask: &mut Vec<DrawKey>) {
        let node = &self.nodes[node];
        if !node.bounds.overlaps(volume.bounds) {
            return;
        }
        match node.children {
            Children::Leaf(index) => {
                #[cfg(test)]
                {
                    self.tested_parts.set(self.tested_parts.get() + 1);
                }
                let part = &self.parts[index];
                if part
                    .bones
                    .iter()
                    .any(|corners| volume.intersects(&corners.map(|corner| corner.as_dvec3())))
                {
                    mask.push(part.key);
                }
            }
            Children::Branch(children) => {
                for child in children {
                    self.query(child, volume, mask);
                }
            }
        }
    }
}

/// One bound per drawn bone is conservative for disjoint cubes and transparent skin texels.
fn rest_bounds(vertices: &[render_model::ActorRigVertex]) -> Option<Arc<RestMesh>> {
    let mut bones = BTreeMap::<usize, Bounds>::new();
    for vertex in vertices {
        let point = Vec3::from_array(vertex.position).as_dvec3();
        let bounds = Bounds::points(&[point])?;
        let index = vertex.bone_index as usize;
        if index >= render_model::MAX_RENDER_BONES_PER_ACTOR {
            return None;
        }
        bones
            .entry(index)
            .and_modify(|old| *old = old.union(bounds))
            .or_insert(bounds);
    }
    let bones = bones
        .into_iter()
        .map(|(index, bounds)| RestBone { index, bounds })
        .collect();
    let bridges = vertices.chunks_exact(3).any(|triangle| {
        triangle[1..]
            .iter()
            .any(|vertex| vertex.bone_index != triangle[0].bone_index)
    });
    Some(Arc::new(RestMesh { bones, bridges }))
}

/// Matches the shader's row-affine skinning and world placement, including pivot, scale and posture.
fn transform(rows: [[f32; 4]; 3], point: Vec3) -> Vec3 {
    let point = point.extend(1.0);
    Vec3::from_array(rows.map(|row| Vec4::from_array(row).dot(point)))
}

/// Median partitions keep query depth logarithmic and retain deterministic ties by source index.
fn build(parts: &[Part], indices: &mut [usize], nodes: &mut Vec<Node>) -> usize {
    let bounds = indices
        .iter()
        .map(|&index| parts[index].bounds)
        .reduce(Bounds::union)
        .expect("nonempty branch");
    let node = nodes.len();
    nodes.push(Node {
        bounds,
        children: Children::Leaf(indices[0]),
    });
    if indices.len() > 1 {
        let extent = bounds.max - bounds.min;
        let axis = if extent.y > extent.x { 1 } else { 0 };
        let axis = if extent.z > extent[axis] { 2 } else { axis };
        indices.sort_unstable_by(|&a, &b| {
            let center =
                |index: usize| parts[index].bounds.min[axis] + parts[index].bounds.max[axis];
            center(a).total_cmp(&center(b)).then(a.cmp(&b))
        });
        let (left, right) = indices.split_at_mut(indices.len() / 2);
        let children = [build(parts, left, nodes), build(parts, right, nodes)];
        nodes[node].children = Children::Branch(children);
    }
    node
}
