// Vertex pulling for shader packs (`pull.rs`): one Iris vertex decoded straight from Cinnabar's
// packed arenas with the expansion kernels' own decode, for a face ref instead of an expanded
// vertex. `geometry/jobs.rs` owns the face-ref encoding.
import render::shaderpack::geometry::expand_common::{IrisVertex, JobFace, job_slot_vertex};
import render::shaderpack::geometry::expand_cubes::cube_face;
import render::shaderpack::geometry::expand_models::model_face;
import render::shaderpack::geometry::expand_liquids::liquid_face;

/// Corner `corner` of the face slot `face_ref` names: x = record, y = origin slot, z = the job's
/// covered block and reserved-face flags, w = the slot within the job | the kernel kind << 8
/// (cube, model, liquid).
fn cnb_pull_vertex(face_ref: vec4<u32>, corner: u32) -> IrisVertex {
    let job = vec4(face_ref.x, face_ref.y, 0u, face_ref.z);
    let kind = face_ref.w >> 8u;
    var face: JobFace;
    if (kind == 0u) {
        face = cube_face(job);
    } else if (kind == 1u) {
        face = model_face(job);
    } else {
        face = liquid_face(job);
    }
    return job_slot_vertex(job, face_ref.w & 0xffu, face, corner);
}
