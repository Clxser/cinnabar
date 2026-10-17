use super::*;

fn plan(faces: u32) -> SectionPlan {
    SectionPlan {
        faces,
        categories: [0..faces, faces..faces, faces..faces],
        jobs: Default::default(),
    }
}

fn entity(index: u32) -> Entity {
    Entity::from_raw_u32(index).unwrap()
}

#[test]
fn pages_grow_by_doubling_then_open_new_pages_within_the_budget() {
    let page_faces = INITIAL_PAGE_FACES * 4;
    let mut residency = Residency::new(
        u64::from(page_faces) * FACE_BYTES,
        6 * u64::from(INITIAL_PAGE_FACES) * FACE_BYTES,
    );
    let first = residency
        .place(entity(1), &plan(INITIAL_PAGE_FACES))
        .unwrap();
    assert_eq!(
        first,
        Placement {
            page: 0,
            faces: 0..INITIAL_PAGE_FACES,
            grown_from: Some(0)
        }
    );
    residency.commit(entity(1), &first, &plan(INITIAL_PAGE_FACES), [0; 3]);
    let second = residency
        .place(entity(2), &plan(INITIAL_PAGE_FACES))
        .unwrap();
    assert_eq!(
        second.grown_from,
        Some(INITIAL_PAGE_FACES),
        "the page doubles in place"
    );
    assert_eq!(second.faces, INITIAL_PAGE_FACES..2 * INITIAL_PAGE_FACES);
    residency.commit(entity(2), &second, &plan(INITIAL_PAGE_FACES), [0; 3]);
    let third = residency
        .place(entity(3), &plan(2 * INITIAL_PAGE_FACES))
        .unwrap();
    assert_eq!(third.page, 0, "page grows to its binding limit");
    assert_eq!(residency.pages[0].faces.capacity(), page_faces);
    residency.commit(entity(3), &third, &plan(2 * INITIAL_PAGE_FACES), [0; 3]);
    let fourth = residency
        .place(entity(4), &plan(INITIAL_PAGE_FACES))
        .unwrap();
    assert_eq!(fourth.page, 1, "a full page opens another");
    residency.commit(entity(4), &fourth, &plan(INITIAL_PAGE_FACES), [0; 3]);
    assert_eq!(
        residency.place(entity(5), &plan(2 * INITIAL_PAGE_FACES)),
        None,
        "the budget is spent"
    );
    assert_eq!(
        residency.pages[1].faces.capacity(),
        INITIAL_PAGE_FACES,
        "a failed placement grows nothing"
    );
    residency.remove(entity(3));
    assert!(
        residency
            .place(entity(5), &plan(2 * INITIAL_PAGE_FACES))
            .is_some(),
        "removed faces are reused"
    );
}

#[test]
fn a_replacement_that_does_not_fit_keeps_the_old_faces() {
    let mut residency = Residency::new(
        u64::from(INITIAL_PAGE_FACES) * FACE_BYTES,
        u64::from(INITIAL_PAGE_FACES) * FACE_BYTES,
    );
    let placement = residency.place(entity(1), &plan(10)).unwrap();
    residency.commit(entity(1), &placement, &plan(10), [0; 3]);
    assert_eq!(
        residency.place(entity(1), &plan(INITIAL_PAGE_FACES + 1)),
        None
    );
    assert_eq!(residency.sections[&entity(1)].faces, 0..10);
    assert!(
        residency
            .place(entity(2), &plan(INITIAL_PAGE_FACES - 10))
            .is_some(),
        "only the old faces stay taken"
    );
}

#[test]
fn the_origin_follows_the_camera_on_a_grid_and_counts_epochs() {
    let mut residency = Residency::new(1 << 20, 1 << 20);
    assert!(residency.follow_camera(DVec3::new(700.0, 64.0, -1600.0)));
    assert_eq!(residency.origin, Some(IVec3::new(1024, 0, -2048)));
    assert!(!residency.follow_camera(DVec3::new(3000.0, 64.0, -2048.0)));
    assert!(residency.follow_camera(DVec3::new(6000.0, 64.0, -2048.0)));
    assert_eq!(residency.origin, Some(IVec3::new(6144, 0, -2048)));
    assert_eq!(residency.epoch, 2);
}

#[test]
fn the_queue_holds_each_section_once_and_forgets_removed_ones() {
    let mut residency = Residency::new(1 << 20, 1 << 20);
    residency.enqueue_all([entity(1), entity(2), entity(1)]);
    residency.remove(entity(2));
    assert_eq!(residency.queue_len(), 1);
    assert_eq!(residency.pop(), Some(entity(1)));
    assert_eq!(residency.pop(), None);
}
