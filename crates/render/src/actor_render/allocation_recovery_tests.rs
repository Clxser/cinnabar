//! Recovery retains immutable publication identities; malformed input remains rejected.

use super::allocation_fault_tests::{draw_frame, fixture, publication};
use super::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// Records every real checked operation at one allocation, upload or view boundary.
fn fault(label: &'static str) -> (impl Drop, Rc<Cell<usize>>) {
    let hits = Rc::new(Cell::new(0));
    let observed = Rc::clone(&hits);
    let guard = crate::gpu_allocation::inject(move |owner, _| {
        if owner != label {
            return false;
        }
        observed.set(observed.get() + 1);
        true
    });
    (guard, hits)
}

#[test]
fn exact_skin_residency_recovers_each_allocation_upload_view_and_copy_failure() {
    for label in [
        "player skin class array",
        "actor skin array view",
        "actor skin upload",
        "actor skin copy",
        "actor skin transfers",
    ] {
        let (device, queue) = wgpu::Device::noop(&Default::default());
        let device = RenderDevice::from(device);
        let queue = RenderQueue::new(queue);
        let mut gpu = super::super::GpuSkinArrays::new(&device).unwrap();
        let first = one_resident_skin();
        gpu.sync(&first, &device, &queue);
        assert!(gpu.is_synced(&first));
        let previous = gpu.view(0).id();
        let mut next = (*first).clone();
        next.classes[0] = Arc::from([first.classes[0][0].clone(), first.classes[0][0].clone()]);
        let next = Arc::new(next);
        let (guard, hits) = fault(label);
        gpu.sync(&next, &device, &queue);
        assert!(
            hits.get() != 0,
            "{label} must reach the real checked boundary"
        );
        assert!(!gpu.is_synced(&next));
        assert!(
            !gpu.rejected(&next),
            "OOM is not immutable source rejection"
        );
        assert_eq!(gpu.view(0).id(), previous);
        drop(guard);
        gpu.sync(&next, &device, &queue);
        assert!(gpu.is_synced(&next), "{label}: exact same Arc must recover");
        let (guard, hits) = fault(label);
        gpu.sync(&next, &device, &queue);
        assert_eq!(
            hits.get(),
            0,
            "warm recovered residency does no transfer work"
        );
        drop(guard);
    }
}

#[test]
fn persistent_skin_pressure_backs_off_without_poisoning_the_source() {
    let (device, queue) = wgpu::Device::noop(&Default::default());
    let device = RenderDevice::from(device);
    let queue = RenderQueue::new(queue);
    let mut gpu = super::super::GpuSkinArrays::new(&device).unwrap();
    let source = one_resident_skin();
    let (guard, hits) = fault("player skin class array");
    gpu.sync(&source, &device, &queue);
    gpu.sync(&source, &device, &queue);
    assert_eq!(hits.get(), 2);
    gpu.sync(&source, &device, &queue);
    assert_eq!(
        hits.get(),
        2,
        "persistent failure skips an allocation opportunity"
    );
    drop(guard);
    gpu.sync(&source, &device, &queue);
    assert!(gpu.is_synced(&source));
}

#[test]
fn blocked_skin_admission_does_not_reupload_actor_buffers_during_backoff() {
    let mut app = fixture();
    let mut next = generic_frame();
    next.instance_pages = Arc::from([0]);
    next.skins = one_resident_skin();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&calls);
    let guard = crate::gpu_allocation::inject(move |owner, _| {
        observed.borrow_mut().push(owner.to_owned());
        owner == "player skin class array"
    });
    assert!(draw_frame(&mut app, &next).is_empty());
    assert!(draw_frame(&mut app, &next).is_empty());
    assert!(
        calls.borrow().iter().all(|owner| {
            matches!(
                owner.as_str(),
                "player skin class array" | "actor skin transfers"
            )
        }),
        "blocked skins cannot trigger geometry or instance uploads"
    );
    let attempts = calls.borrow().len();
    assert!(attempts > 0);
    assert!(draw_frame(&mut app, &next).is_empty());
    assert_eq!(
        calls.borrow().len(),
        attempts,
        "a skipped retry performs no checked GPU work"
    );
    drop(guard);
    assert_eq!(draw_frame(&mut app, &next).len(), 1);
}

#[test]
fn immutable_invalid_skin_framing_and_device_limits_stay_rejected() {
    let (device, queue) = wgpu::Device::noop(&Default::default());
    let device = RenderDevice::from(device);
    let queue = RenderQueue::new(queue);
    for kind in 0..3 {
        let mut gpu = super::super::GpuSkinArrays::new(&device).unwrap();
        let mut invalid = (*one_resident_skin()).clone();
        match kind {
            0 => {
                invalid.classes[0] =
                    vec![None; device.limits().max_texture_array_layers as usize + 1].into()
            }
            1 => {
                Arc::make_mut(&mut invalid.classes[0])[0]
                    .as_mut()
                    .unwrap()
                    .texels = Arc::from([0; 4])
            }
            _ => {
                Arc::make_mut(&mut invalid.classes[0])[0]
                    .as_mut()
                    .unwrap()
                    .admission = 0
            }
        }
        let invalid = Arc::new(invalid);
        let (guard, hits) = fault("actor skin transfers");
        for _ in 0..128 {
            gpu.sync(&invalid, &device, &queue);
            assert!(gpu.rejected(&invalid));
            assert!(!gpu.is_synced(&invalid));
        }
        assert_eq!(hits.get(), 0, "invalid data never reaches GPU admission");
        drop(guard);
        let valid = one_resident_skin();
        gpu.sync(&valid, &device, &queue);
        assert!(
            gpu.is_synced(&valid),
            "rejection must not poison a replacement source"
        );
    }
}

#[test]
fn malformed_geometry_is_acknowledged_once_without_gpu_work() {
    let mut app = fixture();
    let control = publication(1, &[20], 1);
    assert_eq!(draw_frame(&mut app, &control).len(), 1);
    for kind in 0..4 {
        let mut invalid = publication(2 + kind, &[60], 1);
        match kind {
            0 => invalid.rig.geometry_spans = Arc::from([]),
            1 => Arc::make_mut(&mut invalid.rig.geometry_spans)[0].first_vertex = u32::MAX,
            2 => invalid.rig.geometry_vertices.offsets = Arc::from([usize::MAX]),
            _ => {
                let device = app.sub_app(RenderApp).world().resource::<RenderDevice>();
                let len = crate::actor::gpu::storage_binding_bytes(device) as usize
                    / std::mem::size_of::<render_model::ActorRigVertex>()
                    + 1;
                invalid.rig.geometry_vertices =
                    render_model::ActorRigVertexSegments::from_recorded_pages(
                        invalid.rig.geometry_vertices.epoch,
                        Arc::from([]),
                        Arc::from([]),
                        len,
                    )
                    .unwrap();
            }
        }
        let (guard, hits) = fault("shared actor rig vertices");
        for _ in 0..4 {
            assert!(draw_frame(&mut app, &invalid).is_empty());
            let gpu = app
                .sub_app(RenderApp)
                .world()
                .resource::<super::super::ActorGpu>();
            assert_eq!(gpu.geometry_revision, invalid.rig.geometry_revision);
            assert!(!gpu.geometry_ready);
        }
        assert_eq!(hits.get(), 0);
        drop(guard);
    }
    assert_eq!(draw_frame(&mut app, &publication(10, &[100], 1)).len(), 1);
}

#[test]
fn persistent_buffer_pressure_backs_off_and_recovers_the_same_generation() {
    for label in [
        "shared actor rig vertices",
        "actor rig transfer",
        "shared actor rig geometry spans",
        "actor instance and bone upload",
    ] {
        let mut app = fixture();
        let next = publication(1, &[20], 1);
        let (guard, hits) = fault(label);
        assert!(draw_frame(&mut app, &next).is_empty());
        assert!(draw_frame(&mut app, &next).is_empty());
        let attempts = hits.get();
        assert!(attempts >= 2);
        assert!(draw_frame(&mut app, &next).is_empty());
        assert_eq!(
            hits.get(),
            attempts,
            "{label}: persistent pressure cannot spin"
        );
        drop(guard);
        assert_eq!(draw_frame(&mut app, &next).len(), 1, "{label}");
    }
}

#[test]
fn a_new_frame_generation_resets_pending_upload_backoff() {
    let mut app = fixture();
    let first = publication(1, &[20], 1);
    let (guard, hits) = fault("actor instance and bone upload");
    assert!(draw_frame(&mut app, &first).is_empty());
    assert!(draw_frame(&mut app, &first).is_empty());
    assert_eq!(hits.get(), 2);
    drop(guard);
    let next = publication(2, &[60], 1);
    assert_eq!(
        draw_frame(&mut app, &next).len(),
        1,
        "new content is not delayed by an old generation's cooldown"
    );
}

#[test]
fn invalid_structural_publications_do_not_retry_uploads() {
    let mut app = fixture();
    let mut invalid = publication(1, &[20], 1);
    invalid.rig.current_bones = Arc::from([]);
    let (guard, hits) = fault("actor instance and bone upload");
    for _ in 0..4 {
        assert!(draw_frame(&mut app, &invalid).is_empty());
        assert_eq!(
            app.sub_app(RenderApp)
                .world()
                .resource::<super::super::ActorGpu>()
                .frame_generation,
            invalid.rig.frame_generation
        );
    }
    assert_eq!(
        hits.get(),
        0,
        "malformed poses never reach the GPU upload boundary"
    );
    drop(guard);
    assert_eq!(draw_frame(&mut app, &publication(2, &[60], 1)).len(), 1);
}

#[test]
fn recovered_glint_rebinds_players_without_generic_artwork_or_new_frame_generation() {
    let mut app = fixture();
    let mut first = generic_frame();
    first.instance_pages = Arc::from([0]);
    first.skins = one_resident_skin();
    first.artwork = Arc::new(render_model::ActorArtworkPages::default());
    assert_eq!(draw_frame(&mut app, &first).len(), 1);
    let glint = app
        .sub_app(RenderApp)
        .world()
        .resource::<super::super::ActorGpu>()
        .artwork
        .glint
        .texture
        .as_ref()
        .unwrap()
        .0
        .id();
    let mut next = first.clone();
    next.artwork = Arc::new((*first.artwork).clone().with_actor_glint(
        render_model::EquipmentRaster {
            width: 1,
            height: 1,
            rgba8: Arc::from([20; 4]),
        },
    ));
    let (guard, hits) = fault("actor glint view");
    assert_eq!(
        draw_frame(&mut app, &next).len(),
        1,
        "compatible old glint remains drawable"
    );
    assert!(hits.get() > 0);
    let gpu = app
        .sub_app(RenderApp)
        .world()
        .resource::<super::super::ActorGpu>();
    assert_eq!(gpu.artwork.glint.texture.as_ref().unwrap().0.id(), glint);
    let binding = gpu.bind_group.as_ref().unwrap().id();
    drop(guard);
    assert_eq!(draw_frame(&mut app, &next).len(), 1);
    let gpu = app
        .sub_app(RenderApp)
        .world()
        .resource::<super::super::ActorGpu>();
    assert_eq!(gpu.frame_generation, next.rig.frame_generation);
    assert_ne!(gpu.bind_group.as_ref().unwrap().id(), binding);
}
