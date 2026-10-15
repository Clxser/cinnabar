use super::*;

fn input() -> UiRenderInput {
    let page = crate::ui_textures::UiTexturePage::owned([1; 2], Arc::from([255_u8; 4])).unwrap();
    UiRenderInput {
        revision: 1,
        viewport_size: [16; 2],
        safe_area: [0; 4],
        vertices: Arc::from([UiRenderVertex::zeroed(); 3]),
        indices: Arc::from([0, 1, 2]),
        batches: Arc::from([UiRenderBatch::new(
            0,
            UiScissor::new(0, 0, 16, 16),
            0,
            3,
            UI_BLEND_ALPHA,
        )]),
        textures: Arc::new(UiRenderTextureArray::new(vec![page], 1).unwrap()),
    }
}

#[test]
fn revisions_over_admitted_buffers_revalidate_no_geometry() {
    let mut scene = UiRenderScene::default();
    let stats = UiRenderStats::default();
    let first = input();
    scene.publish(first.clone(), &stats).unwrap();
    let mut next = first.clone();
    next.revision += 1;
    VALIDATION_WORK.set([0; 3]);
    scene.publish(next.clone(), &stats).unwrap();
    assert_eq!(VALIDATION_WORK.get(), [0; 3]);
    assert!(Arc::ptr_eq(
        &scene.input.as_ref().unwrap().indices,
        &first.indices
    ));
    Arc::make_mut(&mut next.vertices)[0].position[0] = 1.0;
    next.revision += 1;
    VALIDATION_WORK.set([0; 3]);
    scene.publish(next, &stats).unwrap();
    assert_eq!(VALIDATION_WORK.get(), [first.vertices.len(), 0, 0]);
}

#[test]
fn retained_topology_still_checks_vertex_counts_viewports_and_public_replacements() {
    let stats = UiRenderStats::default();
    let mut scene = UiRenderScene::default();
    let first = input();
    scene.publish(first.clone(), &stats).unwrap();
    let mut shorter = first.clone();
    shorter.revision += 1;
    shorter.vertices = Arc::from([UiRenderVertex::zeroed(); 2]);
    assert_eq!(
        scene.publish(shorter, &stats).unwrap_err().reason,
        UiRenderRejectReason::VertexIndexOutOfBounds
    );
    let mut scene = UiRenderScene::default();
    scene.publish(first.clone(), &stats).unwrap();
    let mut smaller = first.clone();
    smaller.revision += 1;
    smaller.viewport_size = [8; 2];
    assert_eq!(
        scene.publish(smaller, &stats).unwrap_err().reason,
        UiRenderRejectReason::InvalidScissor { batch: 0 }
    );
    let mut scene = UiRenderScene::default();
    scene.publish(first.clone(), &stats).unwrap();
    let mut invalid = first;
    Arc::make_mut(&mut invalid.vertices)[0].position[0] = f32::NAN;
    scene.input = Some(Arc::new(invalid.clone()));
    assert_eq!(
        scene.publish(invalid, &stats).unwrap_err().reason,
        UiRenderRejectReason::NonFiniteVertex
    );
}
