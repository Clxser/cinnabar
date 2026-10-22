use super::*;

#[test]
fn queued_frames_charge_unchanged_pixels_once() {
    let mut budget = Budget::default();
    let mut frame = FrozenVisuals::default();
    frame.particles.base = Some(vec![0u8; 24 * 1024 * 1024].into());
    frame.blocks.dynamic_rgba8 = Arc::clone(frame.particles.base.as_ref().unwrap());
    let mut leases = Vec::new();
    let mut owned = 0;
    for _ in 0..24 {
        let (lease, bytes) = budget.retain(&frame).unwrap();
        owned += bytes;
        leases.push(lease);
    }
    assert!(24 * frame.retained_bytes() > MAX_QUEUED_BYTES);
    assert_eq!(
        budget.used.load(Ordering::Acquire),
        24 * 1024 * 1024 + owned
    );
    drop(leases);
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}

#[test]
fn shared_cells_survive_an_atlas_wrapper_change_without_double_charging_pixels() {
    use render_model::{NametagAtlasRect, NametagScene};
    let mut frame = FrozenVisuals::default();
    let pixels: Arc<[u8]> = vec![0u8; 1024].into();
    frame.nametags = Some(Arc::new(NametagScene {
        atlas: vec![NametagAtlasRect {
            cell: [0, 0, 16, 16],
            rgba8: Arc::clone(&pixels),
        }]
        .into(),
        ..Default::default()
    }));
    let mut budget = Budget::default();
    let (first, first_owned) = budget.retain(&frame).unwrap();
    let mut next = (**frame.nametags.as_ref().unwrap()).clone();
    next.atlas = next.atlas.to_vec().into();
    frame.nametags = Some(Arc::new(next));
    let (second, second_owned) = budget.retain(&frame).unwrap();
    let metadata = std::mem::size_of::<NametagAtlasRect>() + std::mem::size_of::<NametagScene>();
    assert_eq!(
        budget.used.load(Ordering::Acquire),
        pixels.len() + metadata * 2 + first_owned + second_owned
    );
    drop((first, second));
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}

#[test]
fn distinct_allocations_still_exhaust_the_original_budget_and_failed_reservations_release_bytes() {
    let mut budget = Budget::default();
    let a: Arc<[u8]> = vec![0u8; 1].into();
    let b: Arc<[u8]> = vec![1u8; 1].into();
    let mut first = Lease::default();
    budget.shared(&a, MAX_QUEUED_BYTES, &mut first).unwrap();
    let mut second = Lease::default();
    assert!(budget.shared(&b, 1, &mut second).is_err());
    assert_eq!(budget.used.load(Ordering::Acquire), MAX_QUEUED_BYTES);
    drop(first);
    budget.shared(&b, 1, &mut second).unwrap();
    assert_eq!(budget.used.load(Ordering::Acquire), 1);
    drop(second);
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}

#[test]
fn rejected_frame_releases_new_shared_charges_without_releasing_other_queued_samples() {
    let mut budget = Budget::default();
    let old: Arc<[u8]> = vec![0u8; 1].into();
    let mut retained = Lease::default();
    budget
        .shared(&old, MAX_QUEUED_BYTES - 4096, &mut retained)
        .unwrap();
    let before = budget.used.load(Ordering::Acquire);
    let mut frame = FrozenVisuals::default();
    frame.particles.base = Some(vec![0u8; 4096].into());
    assert!(budget.retain(&frame).is_err());
    assert_eq!(budget.used.load(Ordering::Acquire), before);
    drop(retained);
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}

#[test]
fn republished_item_catalogs_share_their_child_pixels_across_queued_epochs() {
    use render_model::{DroppedItemModel, DroppedItemSprite};
    let pixels: Arc<[u8]> = vec![0; 64 * 1024 * 1024].into();
    let model = DroppedItemModel::NativeSprite(DroppedItemSprite {
        width: 4096,
        height: 4096,
        rgba8: Arc::clone(&pixels),
    });
    let mut budget = Budget::default();
    let mut leases = Vec::new();
    let mut owned = 0;
    for _ in 0..7 {
        let mut frame = FrozenVisuals::default();
        frame.items = Some(super::super::items::PreparedItems {
            models: vec![model.clone()].into(),
            ..Default::default()
        });
        let (lease, bytes) = budget.retain(&frame).unwrap();
        owned += bytes;
        leases.push(lease);
    }
    assert_eq!(
        budget.used.load(Ordering::Acquire),
        pixels.len() + 7 * std::mem::size_of::<DroppedItemModel>() + owned
    );
    drop(leases);
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}
