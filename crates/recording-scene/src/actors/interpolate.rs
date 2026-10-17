//! Interpolation keeps source draw identities discrete and blends only compatible affine poses.

use std::{collections::BTreeMap, sync::Arc};

use glam::{Mat3, Quat, Vec3};
use recording_studio::archive::ArchiveError;
use render_model::{ActorDrawManifestEntry, ActorGpuInstance, ActorRenderIdentity};

use super::FrozenActors;

type Bone = [[f32; 4]; 3];
type StableIdentity = (u64, i32, u64, u64, u8);
type BoneGroup = (u32, u32, u32, u32);

struct Group {
    instances: Vec<usize>,
    future: Option<usize>,
}

/// Intermediate frames retain the current draw set; births and discrete resource changes wait for their timestamp.
pub(super) fn sample(
    current: &FrozenActors,
    next: &FrozenActors,
    amount: f32,
    cancelled: &impl Fn() -> bool,
) -> Result<FrozenActors, ArchiveError> {
    if cancelled() {
        return Err(ArchiveError::Cancelled);
    }
    if amount <= 0.0 {
        return Ok(current.clone());
    }
    if amount >= 1.0 {
        return Ok(next.clone());
    }
    if current.geometry_identity != next.geometry_identity
        || current.appearance_identity != next.appearance_identity
        || current.sample.coverage != next.sample.coverage
    {
        return Ok(current.clone());
    }
    let source = &current.sample.rig;
    let future = &next.sample.rig;
    let by_identity = future
        .manifest
        .iter()
        .enumerate()
        .map(|(index, entry)| (identity(entry.identity), index))
        .collect::<BTreeMap<_, _>>();
    let mut groups = Vec::<Group>::new();
    let mut group_ids = BTreeMap::<BoneGroup, usize>::new();
    let mut valid = Vec::new();
    for (index, entry) in source.manifest.iter().enumerate() {
        let instance = &source.instances[index];
        let candidate = by_identity
            .get(&identity(entry.identity))
            .copied()
            .filter(|&index| {
                compatible(
                    entry,
                    instance,
                    &future.manifest[index],
                    &future.instances[index],
                ) && current.sample.instance_pages[entry.instance_index as usize]
                    == next.sample.instance_pages[index]
            });
        let key = (
            entry.previous_bone_base,
            entry.current_bone_base,
            entry.bone_count,
            instance.partial_tick.to_bits(),
        );
        let group = *group_ids.entry(key).or_insert_with(|| {
            groups.push(Group {
                instances: Vec::new(),
                future: candidate,
            });
            valid.push(candidate.is_some());
            groups.len() - 1
        });
        if let (Some(first), Some(other)) = (groups[group].future, candidate) {
            let a = &future.manifest[first];
            let b = &future.manifest[other];
            valid[group] &= a.previous_bone_base == b.previous_bone_base
                && a.current_bone_base == b.current_bone_base
                && a.bone_count == b.bone_count
                && future.instances[first].partial_tick == future.instances[other].partial_tick;
        } else {
            valid[group] = false;
        }
        groups[group].instances.push(index);
    }
    let mut previous_owners = vec![None; source.previous_bones.len()];
    let mut current_owners = vec![None; source.current_bones.len()];
    for (group, entries) in groups.iter().enumerate() {
        let entry = &source.manifest[entries.instances[0]];
        reserve(
            &mut previous_owners,
            entry.previous_bone_base,
            entry.bone_count,
            group,
            &mut valid,
        );
        reserve(
            &mut current_owners,
            entry.current_bone_base,
            entry.bone_count,
            group,
            &mut valid,
        );
    }
    if !valid.iter().any(|valid| *valid) {
        return Ok(current.clone());
    }
    let mut output = current.clone();
    let mut instances = source.instances.to_vec();
    let mut compat = current.sample.instances.to_vec();
    let mut previous_bones = source.previous_bones.to_vec();
    let mut current_bones = source.current_bones.to_vec();
    for (_, entries) in groups.iter().enumerate().filter(|(group, _)| valid[*group]) {
        if cancelled() {
            return Err(ArchiveError::Cancelled);
        }
        let first = entries.instances[0];
        let Some(next_index) = entries.future else {
            continue;
        };
        let a = &source.manifest[first];
        let b = &future.manifest[next_index];
        let bones = (0..a.bone_count as usize)
            .map(|bone| {
                let a_pose = evaluated(
                    source.previous_bones[a.previous_bone_base as usize + bone],
                    source.current_bones[a.current_bone_base as usize + bone],
                    source.instances[first].partial_tick,
                );
                let b_pose = evaluated(
                    future.previous_bones[b.previous_bone_base as usize + bone],
                    future.current_bones[b.current_bone_base as usize + bone],
                    future.instances[next_index].partial_tick,
                );
                blend(a_pose, b_pose, amount).unwrap_or(a_pose)
            })
            .collect::<Vec<_>>();
        let transforms = entries
            .instances
            .iter()
            .map(|&index| {
                let other = by_identity[&identity(source.manifest[index].identity)];
                blend(
                    source.instances[index].world_from_actor,
                    future.instances[other].world_from_actor,
                    amount,
                )
                .map(|transform| (index, other, transform))
            })
            .collect::<Option<Vec<_>>>();
        let Some(transforms) = transforms else {
            continue;
        };
        for (bone, matrix) in bones.into_iter().enumerate() {
            previous_bones[a.previous_bone_base as usize + bone] = matrix;
            current_bones[a.current_bone_base as usize + bone] = matrix;
        }
        for (index, other, transform) in transforms {
            instances[index].world_from_actor = transform;
            instances[index].partial_tick = 0.0;
            let old = &current.sample.instances[index];
            let new = &next.sample.instances[other];
            compat[index].position = std::array::from_fn(|axis| {
                old.position[axis] + (new.position[axis] - old.position[axis]) * amount
            });
            compat[index].yaw_radians = angle(old.yaw_radians, new.yaw_radians, amount);
            compat[index].pitch_radians =
                old.pitch_radians + (new.pitch_radians - old.pitch_radians) * amount;
            compat[index].head_yaw_radians =
                angle(old.head_yaw_radians, new.head_yaw_radians, amount);
        }
    }
    output.sample.instances = compat.into();
    output.sample.rig.instances = instances.into();
    output.sample.rig.previous_bones = Arc::from(previous_bones);
    output.sample.rig.current_bones = Arc::from(current_bones);
    Ok(output)
}

/// Animation and ingress revisions can advance while a recorded actor's lifetime and draw layer stay stable.
fn identity(value: ActorRenderIdentity) -> StableIdentity {
    (
        value.session_id,
        value.dimension,
        value.runtime_id,
        value.spawn_revision,
        value.layer,
    )
}

/// Teleport/reset, rig, material and texture changes retain their recorded boundary rather than morphing.
fn compatible(
    a: &ActorDrawManifestEntry,
    a_instance: &ActorGpuInstance,
    b: &ActorDrawManifestEntry,
    b_instance: &ActorGpuInstance,
) -> bool {
    a.rig == b.rig
        && a.route == b.route
        && a.bone_count == b.bone_count
        && a.reset_generation == b.reset_generation
        && a_instance.reset_generation == b_instance.reset_generation
        && a_instance.geometry_id == b_instance.geometry_id
        && a_instance.texture_layer == b_instance.texture_layer
        && a_instance.material == b_instance.material
        && a_instance.multitexture_layers == b_instance.multitexture_layers
}

/// Shared exact pose ranges blend once; partially overlapping ranges remain unchanged as one group.
fn reserve(owners: &mut [Option<usize>], base: u32, count: u32, group: usize, valid: &mut [bool]) {
    for owner in &mut owners[base as usize..(base + count) as usize] {
        if let Some(other) = *owner
            && other != group
        {
            valid[other] = false;
            valid[group] = false;
        } else {
            *owner = Some(group);
        }
    }
}

/// The captured GPU shader evaluates a source sample by blending its two prepared bone matrices.
fn evaluated(a: Bone, b: Bone, amount: f32) -> Bone {
    std::array::from_fn(|row| {
        std::array::from_fn(|column| a[row][column] + (b[row][column] - a[row][column]) * amount)
    })
}

/// Polar rotation plus residual stretch preserves authored affine shear while rotation takes the shorter arc.
fn blend(a: Bone, b: Bone, amount: f32) -> Option<Bone> {
    let (a_rotation, a_stretch, a_translation) = decompose(a)?;
    let (b_rotation, b_stretch, b_translation) = decompose(b)?;
    let rotation = Mat3::from_quat(a_rotation.slerp(b_rotation, amount));
    let matrix = rotation * (a_stretch * (1.0 - amount) + b_stretch * amount);
    let translation = a_translation.lerp(b_translation, amount);
    let columns = matrix.to_cols_array_2d();
    let result = std::array::from_fn(|row| {
        [
            columns[0][row],
            columns[1][row],
            columns[2][row],
            translation[row],
        ]
    });
    result
        .iter()
        .flatten()
        .all(|value| value.is_finite())
        .then_some(result)
}

/// Singular or extreme transforms retain the current admitted pose rather than emitting an invalid matrix.
fn decompose(value: Bone) -> Option<(Quat, Mat3, Vec3)> {
    let linear = Mat3::from_cols(
        Vec3::new(value[0][0], value[1][0], value[2][0]),
        Vec3::new(value[0][1], value[1][1], value[2][1]),
        Vec3::new(value[0][2], value[1][2], value[2][2]),
    );
    let scale = linear
        .x_axis
        .length()
        .max(linear.y_axis.length())
        .max(linear.z_axis.length());
    if !scale.is_finite() || scale <= f32::MIN_POSITIVE {
        return None;
    }
    let mut rotation = linear * scale.recip();
    for _ in 0..12 {
        let determinant = rotation.determinant();
        if !determinant.is_finite() || determinant.abs() < 1e-12 {
            return None;
        }
        let next = (rotation + rotation.inverse().transpose()) * 0.5;
        let error = (next - rotation)
            .to_cols_array()
            .into_iter()
            .fold(0.0_f32, |error, value| error.max(value.abs()));
        rotation = next;
        if error <= 1e-5 {
            break;
        }
    }
    let orthogonal = rotation.transpose() * rotation;
    if !rotation.is_finite()
        || (orthogonal - Mat3::IDENTITY)
            .to_cols_array()
            .into_iter()
            .any(|value| value.abs() > 1e-3)
    {
        return None;
    }
    if rotation.determinant() < 0.0 {
        rotation.x_axis = -rotation.x_axis;
    }
    let quaternion = Quat::from_mat3(&rotation).normalize();
    if !quaternion.is_finite() {
        return None;
    }
    Some((
        quaternion,
        rotation.transpose() * linear,
        Vec3::new(value[0][3], value[1][3], value[2][3]),
    ))
}

/// Compatibility actor headings use the same short-arc convention as the rendered affine rotation.
fn angle(a: f32, b: f32, amount: f32) -> f32 {
    a + ((b - a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI)
        * amount
}
