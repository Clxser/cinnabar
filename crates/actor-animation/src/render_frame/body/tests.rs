use super::*;

/// Finite model-space transforms exercise both pixel translation and independent per-axis scale.
fn bones(count: usize, translation: f32) -> Arc<[BoneTransform]> {
    vec![
        BoneTransform {
            rotation: [0.0, 0.0, 0.0, 1.0],
            translation_scale: [translation, 0.0, 0.0, 1.0],
            axis_scale: [1.0, 2.0, 3.0],
        };
        count
    ]
    .into()
}

/// An immutable body layer uses exactly the native publication fields consumed by the bridge.
fn layer(previous: Arc<[BoneTransform]>, current: Arc<[BoneTransform]>) -> RenderTextureLayer {
    RenderTextureLayer {
        material: Default::default(),
        material_state: None,
        source: 0,
        texture_slot: 0,
        multitexture: None,
        color: [1.0; 4],
        overlay: [0.0; 4],
        hidden_bones: Arc::from([]),
        uv_anim: [0.0, 0.0, 1.0, 1.0],
        geometry: None,
        previous_pose: previous,
        pose: current,
        ignore_lighting: false,
        light_color_multiplier: 1.0,
        sampled_scale: None,
    }
}

/// Synthetic persona storage makes unchanged raster and layer ownership observable without carriers.
fn skin(pixels: Arc<[u8]>) -> SkinRenderLayer {
    let pose = bones(1, 0.0);
    SkinRenderLayer {
        image: render_api::SkinAnimation {
            kind: render_api::SkinAnimationKind::Face,
            width: 1,
            height: 1,
            rgba8: pixels,
            frames: 1,
            blinking: false,
        },
        geometry: Arc::new(assets::SkinGeometry {
            identifier: "geometry.fixture".into(),
            texture_width: 1,
            texture_height: 1,
            bones: Box::new([]),
            poly_meshes: Box::new([]),
            visible_bounds: None,
            animation_no_head_bob: false,
            digest: [0; 32],
        }),
        mesh: None,
        previous: Arc::clone(&pose),
        current: Arc::clone(&pose),
        rest: pose,
        hidden_bones: Arc::from([]),
        uv_anim: [0.0, 0.0, 1.0, 1.0],
    }
}

#[test]
fn repeated_body_consumers_convert_once_and_clone_only_shared_handles() {
    let pixels: Arc<[u8]> = vec![255; 4].into();
    let layers = ActorRenderLayers {
        render: Cow::Owned(vec![layer(bones(2, 8.0), bones(2, 16.0))]),
        skin: Cow::Owned(vec![skin(Arc::clone(&pixels))]),
        held_item_scale: Some(1.0),
    };
    let mut cache = Cache::default();
    let first = cache.sample(&layers, [2, 2]).unwrap();
    assert_eq!(cache.endpoints, 2);
    assert_eq!(first.bones[0][0].translation_scale[0], 0.5);
    assert_eq!(first.bones[1][0].translation_scale[0], 1.0);
    assert_eq!(first.bones[1][0].axis_scale, [1.0, 2.0, 3.0, 1.0]);
    let (_, allocations) = test_alloc::measure(|| {
        for _ in 0..64 {
            let next = cache.sample(&layers, [2, 2]).unwrap();
            assert!(Arc::ptr_eq(&first.bones[0], &next.bones[0]));
            assert!(Arc::ptr_eq(&first.bones[1], &next.bones[1]));
            assert!(Arc::ptr_eq(
                first.skin.as_ref().unwrap(),
                next.skin.as_ref().unwrap()
            ));
        }
    });
    assert_eq!(allocations.calls, 0);
    assert_eq!(cache.endpoints, 2);
    assert!(Arc::ptr_eq(
        &first.skin.as_ref().unwrap()[0].image.rgba8,
        &pixels
    ));
}

#[test]
fn endpoint_aliases_and_invalid_bones_do_not_repeat_conversion() {
    let pose = bones(1, 4.0);
    let layers = ActorRenderLayers {
        render: Cow::Owned(vec![layer(Arc::clone(&pose), pose)]),
        skin: Cow::Borrowed(&[]),
        held_item_scale: None,
    };
    let mut cache = Cache::default();
    let sample = cache.sample(&layers, [1, 1]).unwrap();
    assert!(Arc::ptr_eq(&sample.bones[0], &sample.bones[1]));
    assert!(sample.skin.is_none());
    assert_eq!(cache.endpoints, 1);
    let invalid = ActorRenderLayers {
        render: Cow::Owned(vec![layer(bones(1, 0.0), bones(1, f32::NAN))]),
        skin: Cow::Borrowed(&[]),
        held_item_scale: None,
    };
    let mut cache = Cache::default();
    for _ in 0..2 {
        assert!(cache.sample(&invalid, [1, 1]).is_none());
    }
    assert_eq!(cache.endpoints, 1);
}

#[test]
fn completed_tick_layers_keep_the_existing_tick_owned_pose_conversion() {
    let render = [layer(bones(1, 0.0), bones(1, 16.0))];
    let layers = ActorRenderLayers {
        render: Cow::Borrowed(&render),
        skin: Cow::Borrowed(&[]),
        held_item_scale: None,
    };
    let mut cache = Cache::default();
    assert!(cache.sample(&layers, [1, 1]).is_none());
    assert_eq!(cache.endpoints, 0);
}

#[test]
fn skeleton_lengths_select_the_correct_layer_without_admitting_alternate_geometry() {
    let mut alternate = layer(bones(3, 12.0), bones(3, 12.0));
    alternate.geometry = Some(77);
    let layers = ActorRenderLayers {
        render: Cow::Owned(vec![
            alternate,
            layer(bones(1, 8.0), bones(1, 16.0)),
            layer(bones(2, 24.0), bones(2, 32.0)),
        ]),
        skin: Cow::Borrowed(&[]),
        held_item_scale: None,
    };
    let mut cache = Cache::default();
    assert!(cache.sample(&layers, [3, 3]).is_none());
    assert!(cache.sample(&layers, [1, 2]).is_none());
    let first = cache.sample(&layers, [1, 1]).unwrap();
    let second = cache.sample(&layers, [2, 2]).unwrap();
    assert_eq!(first.bones[1].len(), 1);
    assert_eq!(second.bones[1].len(), 2);
    assert_eq!(second.bones[1][0].translation_scale[0], 2.0);
    assert_eq!(cache.endpoints, 4);
}

#[test]
fn frame_cache_replacement_releases_old_endpoints_and_persona_storage() {
    let pixels: Arc<[u8]> = vec![255; 4].into();
    let layers = ActorRenderLayers {
        render: Cow::Owned(vec![layer(bones(1, 0.0), bones(1, 16.0))]),
        skin: Cow::Owned(vec![skin(Arc::clone(&pixels))]),
        held_item_scale: None,
    };
    let mut cache = Cache::default();
    let first = cache.sample(&layers, [1, 1]).unwrap();
    let old_bones = Arc::downgrade(&first.bones[1]);
    let old_skin = Arc::downgrade(first.skin.as_ref().unwrap());
    drop((first, layers));
    let next_pixels: Arc<[u8]> = vec![17; 4].into();
    let layers = ActorRenderLayers {
        render: Cow::Owned(vec![layer(bones(2, 8.0), bones(2, 32.0))]),
        skin: Cow::Owned(vec![skin(Arc::clone(&next_pixels))]),
        held_item_scale: None,
    };
    cache = Cache::default();
    assert!(old_bones.upgrade().is_none());
    assert!(old_skin.upgrade().is_none());
    assert_eq!(Arc::strong_count(&pixels), 1);
    let next = cache.sample(&layers, [2, 2]).unwrap();
    assert_eq!(next.bones[1].len(), 2);
    assert_eq!(next.bones[1][0].translation_scale[0], 2.0);
    assert!(Arc::ptr_eq(
        &next.skin.as_ref().unwrap()[0].image.rgba8,
        &next_pixels
    ));
    let retained = Arc::downgrade(&next.bones[1]);
    drop(cache);
    assert!(
        retained.upgrade().is_some(),
        "consumer retains its own immutable sample"
    );
    drop(next);
    assert!(retained.upgrade().is_none());
}

#[test]
fn changing_local_view_or_retrying_an_exhausted_sample_drops_its_converted_body() {
    let store = ActorAnimationStore::diagnostic();
    let actors = std::collections::HashMap::<u64, crate::TestActor>::new();
    for exhausted in [false, true] {
        let mut frame = ActorRenderFrame::new(&store, &actors, [0.0; 2], [0.0; 3], 0.5);
        frame.sampled_layers.insert(
            7,
            CachedLayers {
                layers: Some(ActorRenderLayers {
                    render: Cow::Owned(vec![layer(bones(1, 0.0), bones(1, 16.0))]),
                    skin: Cow::Borrowed(&[]),
                    held_item_scale: None,
                }),
                body: Cache::default(),
                exhausted,
            },
        );
        let sample = frame.body_with_skin(7, 1, 1).unwrap();
        let previous = Arc::downgrade(&sample.bones[1]);
        drop(sample);
        let mut next = if exhausted {
            frame.into_additional_view()
        } else {
            frame.with_world_body(7)
        };
        assert!(previous.upgrade().is_none());
        // No actor is installed: neither an invalidated local view nor an exhausted result
        // can leak the preceding consumer's body into this independently sampled view.
        assert!(next.body_with_skin(7, 1, 1).is_none());
    }
}
