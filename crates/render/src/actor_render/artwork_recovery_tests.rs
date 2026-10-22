use super::*;
use std::{cell::Cell, rc::Rc};

/// Keeps fault counts scoped to the owning resource rather than unrelated placeholders.
fn fault(label: &'static str) -> (impl Drop, Rc<Cell<usize>>) {
    let hits = Rc::new(Cell::new(0));
    let observed = Rc::clone(&hits);
    let guard = gpu_allocation::inject(move |owner, _| {
        if owner != label {
            return false;
        }
        observed.set(observed.get() + 1);
        true
    });
    (guard, hits)
}

#[test]
fn first_glint_allocation_upload_and_view_failures_recover_without_new_identity() {
    for label in [
        "immutable actor glint",
        "immutable item glint",
        "actor page upload",
        "actor glint view",
    ] {
        let (device, queue) = wgpu::Device::noop(&Default::default());
        let device = RenderDevice::from(device);
        let queue = RenderQueue::new(queue);
        let pages = ActorArtworkPages::default();
        let mut gpu = GpuArtwork::default();
        let (guard, hits) = fault(label);
        assert!(gpu.prepare(&pages, &device, &queue));
        assert!(hits.get() > 0, "{label}");
        assert!(gpu.glint.texture.is_none() || gpu.item_glint.texture.is_none());
        drop(guard);
        assert!(gpu.prepare(&pages, &device, &queue));
        assert!(
            gpu.glint.texture.is_some() && gpu.item_glint.texture.is_some(),
            "{label}"
        );
        assert!(gpu.glints_changed);
        let (guard, hits) = fault(label);
        assert!(gpu.prepare(&pages, &device, &queue));
        assert!(!gpu.glints_changed);
        assert_eq!(
            hits.get(),
            0,
            "recovered glints are warm without reallocation"
        );
        drop(guard);
    }
}

#[test]
fn persistent_glint_pressure_backs_off_and_recovers_without_rehashing() {
    let (device, queue) = wgpu::Device::noop(&Default::default());
    let device = RenderDevice::from(device);
    let queue = RenderQueue::new(queue);
    let mut gpu = GpuArtwork::default();
    let pages = ActorArtworkPages::default().with_actor_glint(render_model::EquipmentRaster {
        width: 4,
        height: 4,
        rgba8: Arc::from([13; 4 * 4 * 4]),
    });
    let (guard, hits) = fault("immutable actor glint");
    assert!(gpu.prepare(&pages, &device, &queue));
    let attempts = hits.get();
    assert_eq!(attempts, 3, "fresh glints reduce through 4x4, 2x2 and 1x1");
    let hashed = work_tests::hashed_bytes();
    assert!(gpu.prepare(&pages, &device, &queue));
    assert_eq!(hits.get(), attempts * 2);
    assert_eq!(
        work_tests::hashed_bytes() - hashed,
        0,
        "retained source keys and the shared fallback avoid repeated hashing"
    );
    assert!(gpu.prepare(&pages, &device, &queue));
    assert_eq!(hits.get(), attempts * 2);
    drop(guard);
    assert!(gpu.prepare(&pages, &device, &queue));
    assert!(gpu.glint.texture.is_some());
}

#[test]
fn first_demanded_page_faults_recover_with_the_exact_same_pixels_and_identity() {
    for label in [
        "immutable actor page",
        "actor page upload",
        "actor page view",
    ] {
        let (device, queue) = wgpu::Device::noop(&Default::default());
        let device = RenderDevice::from(device);
        let queue = RenderQueue::new(queue);
        let mut gpu = GpuArtwork::default();
        assert!(gpu.prepare(&ActorArtworkPages::default(), &device, &queue));
        let mut pages = ActorArtworkPages::default();
        pages.entity_identity = [1; 32];
        pages.pages = Arc::from([render_model::ActorTexturePage {
            width: 1,
            height: 1,
            layers: 1,
            rgba8: Arc::from([20; 4]),
            color_mask: false,
            multitexture: false,
        }]);
        let (guard, hits) = fault(label);
        assert!(!gpu.prepare_demanded(&pages, &[1], &device, &queue));
        assert!(hits.get() > 0, "{label}");
        assert!(gpu.page(0).is_none());
        assert!(!gpu.rejected);
        drop(guard);
        assert!(gpu.prepare_demanded(&pages, &[1], &device, &queue));
        assert!(
            gpu.page(0)
                .unwrap()
                .source
                .shares_pixels_with(&pages.pages[0])
        );
        let (guard, hits) = fault(label);
        assert!(gpu.prepare_demanded(&pages, &[1], &device, &queue));
        assert_eq!(hits.get(), 0, "warm recovered demand does no upload work");
        drop(guard);
    }
}

#[test]
fn malformed_and_unsupported_artwork_is_sticky_without_gpu_attempts() {
    let (device, queue) = wgpu::Device::noop(&Default::default());
    let device = RenderDevice::from(device);
    let queue = RenderQueue::new(queue);
    for kind in 0..4 {
        let mut page = render_model::ActorTexturePage {
            width: 1,
            height: 1,
            layers: 1,
            rgba8: Arc::from([0; 4]),
            color_mask: false,
            multitexture: false,
        };
        match kind {
            0 => page.width = 0,
            1 => page.layers = 0,
            2 => page.rgba8 = Arc::from([0; 3]),
            _ => page.layers = device.limits().max_texture_array_layers + 1,
        }
        let mut pages = ActorArtworkPages::default();
        pages.entity_identity = [1; 32];
        pages.pages = Arc::from([page]);
        let mut gpu = GpuArtwork::default();
        let (guard, hits) = fault("immutable actor page");
        for _ in 0..128 {
            assert!(!gpu.prepare(&pages, &device, &queue));
            assert!(gpu.rejected);
            assert!(gpu.pages.is_empty());
        }
        assert_eq!(
            hits.get(),
            0,
            "immutable rejection must not repeatedly allocate"
        );
        drop(guard);
    }
}

#[test]
fn malformed_glint_is_sticky_until_a_replacement_source_arrives() {
    let (device, queue) = wgpu::Device::noop(&Default::default());
    let device = RenderDevice::from(device);
    let queue = RenderQueue::new(queue);
    let mut pages = ActorArtworkPages::default();
    pages.actor_glint = Some(render_model::EquipmentRaster {
        width: 1,
        height: 1,
        rgba8: Arc::from([0; 3]),
    });
    let mut gpu = GpuArtwork::default();
    let (guard, hits) = fault("immutable actor glint");
    for _ in 0..128 {
        assert!(gpu.prepare(&pages, &device, &queue));
        assert!(gpu.glint.pending.as_ref().unwrap().rejected);
    }
    assert_eq!(hits.get(), 0);
    drop(guard);
    let valid = pages.with_actor_glint(render_model::EquipmentRaster {
        width: 1,
        height: 1,
        rgba8: Arc::from([20; 4]),
    });
    assert!(gpu.prepare(&valid, &device, &queue));
    assert!(gpu.glint.texture.is_some());
    assert!(gpu.glint.pending.is_none());
}

#[test]
fn returning_to_the_resident_page_drops_the_failed_replacement_source() {
    let (device, queue) = wgpu::Device::noop(&Default::default());
    let device = RenderDevice::from(device);
    let queue = RenderQueue::new(queue);
    let mut first = ActorArtworkPages::default();
    first.entity_identity = [1; 32];
    first.pages = Arc::from([render_model::ActorTexturePage {
        width: 1,
        height: 1,
        layers: 1,
        rgba8: Arc::from([10; 4]),
        color_mask: false,
        multitexture: false,
    }]);
    let mut next = first.clone();
    next.entity_identity = [2; 32];
    Arc::make_mut(&mut next.pages)[0].rgba8 = Arc::from([20; 4]);
    let weak = Arc::downgrade(&next.pages[0].rgba8);
    let mut gpu = GpuArtwork::default();
    assert!(gpu.prepare(&first, &device, &queue));
    {
        let (_guard, _) = fault("immutable actor page");
        assert!(gpu.prepare(&next, &device, &queue));
    }
    drop(next);
    assert!(weak.upgrade().is_some());
    assert!(gpu.prepare(&first, &device, &queue));
    assert!(
        weak.upgrade().is_none(),
        "obsolete pending pixels lose their last owner"
    );
}
