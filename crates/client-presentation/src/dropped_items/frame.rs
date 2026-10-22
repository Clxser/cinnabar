//! Shared model preparation for native drawing and independent received-item recording.

use super::*;

impl DroppedItemPublisher<'_, '_> {
    /// Owns model admission, native item poses, static contents and source-camera line geometry.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_scene(
        scene: &mut DroppedItemScene,
        cache: &mut ModelCache,
        scratch: &mut PublishScratch,
        textures: Option<&ChunkTextureAssets>,
        placements: &[render_model::StaticItemPlacement],
        device: Option<&bevy::render::renderer::RenderDevice>,
        icons: &UiPresentationRuntime,
        stream: &WorldStream,
        collisions: Option<&dyn crate::observations::CollisionLookup>,
        camera: Option<([f32; 3], f32)>,
        frustum: Option<render_model::dropped_item::DroppedItemFrustum>,
        partial_tick: f32,
        glint: (
            Option<render_model::EquipmentRaster>,
            render_model::ActorGlint,
        ),
        received: bool,
    ) -> recording::Admission {
        let mut admission = recording::Admission::default();
        cache.sync(
            stream.authority().actor_session_id(),
            textures.map(|textures| textures.identity()),
            icons.session_icon_generation(),
        );
        let assets = textures.map(|textures| textures.assets());
        let PublishScratch {
            instances,
            terrain_instances,
            lines,
            placement_icons,
            transitions,
            dropped,
            candidates,
            ropes,
        } = scratch;
        instances.clear();
        terrain_instances.clear();
        placement_icons.clear();

        let item_limit = if received {
            render_model::MAX_DROPPED_ITEM_INSTANCES
        } else {
            usize::MAX
        };
        stream.authority().dropped_items_into(partial_tick, dropped);
        admission.instance_limit = dropped.len() > item_limit;
        dropped.truncate(item_limit);
        let mut dial_models = dials::DialModels::default();
        cache.begin_frame();
        // Protect all current consumers before admitting new models, regardless of iteration order.
        for view in dropped.iter() {
            if let Some(sheet) = view
                .item
                .identifier
                .as_deref()
                .and_then(|id| icons.session_block_sheet(id, view.item.identity.metadata))
            {
                cache.request(&ModelKey::session_block(sheet));
            }
            if let Some((mode, id)) = item_block_id(stream, view.item.visual) {
                let hashed = mode == NetworkIdMode::Hashed;
                cache.request(&ModelKey::CarriedBlock { hashed, id });
                cache.request(&ModelKey::Block { hashed, id });
            }
            if let Some(identifier) = view.item.identifier.as_ref() {
                dial_models.observe(icons, identifier, view.item.identity.metadata);
                let (identifier, metadata) = dropped_icon_key(icons, identifier, &view.item);
                if let Some(icon) = icons.item_icon(identifier, metadata) {
                    cache.request(&ModelKey::icon(icon, true));
                }
            }
        }
        if !placements.is_empty() {
            for placement in placements.iter().take(if received {
                render_model::MAX_DROPPED_ITEM_INSTANCES
            } else {
                usize::MAX
            }) {
                if let Some(id) = placement.potted_block {
                    cache.request(&ModelKey::PottedBlock { id });
                    placement_icons.push(None);
                    continue;
                }
                let icon = icons.item_icon(&placement.identifier, placement.metadata);
                if let Some(icon) = icon {
                    cache.request(&ModelKey::icon(icon, false));
                }
                placement_icons.push(icon);
            }
        }
        stream
            .authority()
            .block_entity_candidates_into(partial_tick, candidates);
        admission.instance_limit |= candidates.len() > item_limit;
        candidates.truncate(item_limit);
        for candidate in candidates.iter() {
            if let Some((mode, id, _)) = entity_block_id(stream, &candidate.view.kind) {
                cache.request(&ModelKey::Block {
                    hashed: mode == NetworkIdMode::Hashed,
                    id,
                });
            }
        }
        cache.finish_demand();
        preparation::retain_lifetimes(cache, dropped);
        for view in dropped.iter() {
            let Some(identifier) = view.item.identifier.as_ref() else {
                continue;
            };
            let block_id = item_block_id(stream, view.item.visual);
            let preparation = DropPreparation {
                view,
                native_origin: stream.authority().actor(view.runtime_id).map_or(
                    view.position,
                    |actor| {
                        let mut origin = actor.position;
                        origin[1] += protocol::ITEM_ACTOR_NETWORK_OFFSET;
                        origin
                    },
                ),
                camera,
                frustum: frustum.as_ref(),
            };
            let session_block = icons
                .session_block_sheet(identifier, view.item.identity.metadata)
                .and_then(|sheet| {
                    preparation.model(
                        cache,
                        &ModelKey::session_block(sheet),
                        DroppedItemShape::Cube,
                        |cache| {
                            cache.model(
                                ModelKey::session_block(sheet),
                                || {
                                    icons.session_block_texture_layers(
                                        identifier,
                                        view.item.identity.metadata,
                                    )
                                },
                                || {
                                    icons.session_block_model(
                                        identifier,
                                        view.item.identity.metadata,
                                        sheet,
                                    )
                                },
                            )
                        },
                    )
                });
            let cube = session_block.or_else(|| {
                block_id.zip(assets).and_then(|((mode, id), assets)| {
                    let hashed = mode == NetworkIdMode::Hashed;
                    preparation
                        .model(
                            cache,
                            &ModelKey::CarriedBlock { hashed, id },
                            DroppedItemShape::Cube,
                            |cache| Self::carried_block_model(cache, icons, assets, mode, id),
                        )
                        .or_else(|| {
                            preparation.model(
                                cache,
                                &ModelKey::Block { hashed, id },
                                DroppedItemShape::Cube,
                                |cache| Self::block_model(cache, assets, mode, id),
                            )
                        })
                })
            });
            let prepared = cube.or_else(|| {
                let (icon_identifier, variant) = dropped_icon_key(icons, identifier, &view.item);
                let key = ModelKey::icon(icons.item_icon(icon_identifier, variant)?, true);
                preparation.model(cache, &key, DroppedItemShape::Sprite, |cache| {
                    Self::icon_model(cache, icons, icon_identifier, variant, true)
                })
            });
            if let Some(prepared) = prepared {
                prepared.append(view, instances, |position| stream.light_level_at(position));
            } else {
                admission.model_failures += 1;
            }
        }

        // Items held by block entities (item frames, campfires) use the same sprite path.
        if !placements.is_empty() {
            for (placement, &icon) in
                placements
                    .iter()
                    .zip(placement_icons.iter())
                    .take(if received {
                        render_model::MAX_DROPPED_ITEM_INSTANCES
                    } else {
                        usize::MAX
                    })
            {
                let model = if let Some(id) = placement.potted_block {
                    assets.and_then(|assets| Self::potted_model(cache, assets, id))
                } else {
                    icon.and_then(|icon| {
                        Self::resolved_icon_model(cache, icon, false, || {
                            icon_sprite(icons, &placement.identifier, placement.metadata, false)
                        })
                    })
                };
                let Some(model) = model else {
                    admission.model_failures += 1;
                    continue;
                };
                let rows = placement.world_from_item;
                let (block_level, sky_level) = placement
                    .light
                    .unwrap_or_else(|| stream.light_level_at([rows[0][3], rows[1][3], rows[2][3]]));
                instances.push(DroppedItemInstance {
                    model,
                    world_from_item: rows,
                    block_level: u32::from(block_level),
                    sky_level: u32::from(sky_level),
                    overlay_rgba8: 0,
                    glint: false,
                });
            }
        }

        if let Some(assets) = assets {
            transitions.clear();
            for fence in stream.actor_block_sync_fences() {
                transitions
                    .entry(fence.sync.actor_unique_id)
                    .or_default()
                    .push(TerrainItemTransition {
                        key: fence.key,
                        generation: fence.generation,
                        visible: fence.sync.message == 1,
                    });
            }
            admission.instance_limit |= received && candidates.len() > item_limit;
            for candidate in candidates.iter() {
                let view = &candidate.view;
                let Some((id_mode, id, base_scale)) = entity_block_id(stream, &view.kind) else {
                    continue;
                };
                let Some(model) = Self::block_model(cache, assets, id_mode, id) else {
                    admission.model_failures += 1;
                    continue;
                };
                let light_position = lighting::block_entity_light_position(
                    stream,
                    collisions,
                    &view.kind,
                    view.center,
                );
                let (block_level, sky_level) = stream.light_level_at(light_position);
                let instance = DroppedItemInstance {
                    model,
                    world_from_item: dropped_item_transform(
                        view.center,
                        0.0,
                        base_scale * view.scale,
                    ),
                    block_level: u32::from(block_level),
                    sky_level: u32::from(sky_level),
                    overlay_rgba8: if view.flash {
                        pack_overlay_rgba8(TNT_FLASH_OVERLAY)
                    } else {
                        0
                    },
                    glint: false,
                };
                if matches!(view.kind, BlockEntityKind::Falling { .. }) {
                    terrain_instances.push(TerrainItemInstance {
                        instance,
                        visible: candidate.visible,
                        transitions: Arc::from(
                            transitions.remove(&candidate.unique_id).unwrap_or_default(),
                        ),
                    });
                } else if candidate.visible {
                    instances.push(instance);
                }
            }
        }

        lines.clear();
        if let Some((camera, _)) = camera {
            stream.authority().ropes_into(partial_tick, ropes);
            for rope in ropes.iter() {
                let rope = recording::prepare_rope(*rope);
                if received && lines.len() >= render_model::MAX_DYNAMIC_ITEM_VERTICES {
                    admission.rope_limit = true;
                    break;
                }
                rope_ribbon(
                    rope.from,
                    rope.to,
                    camera,
                    rope.segments as usize,
                    rope.sag,
                    rope.half_width,
                    rope.color,
                    lines,
                );
            }
        }
        dial_models.warm(cache, icons);
        let (policy, max_array_layers) = device.map_or_else(
            || {
                (
                    assets::texture::policy::device_policy(),
                    render_model::MAX_ITEM_LAYERS,
                )
            },
            |device| {
                let limits = device.limits();
                (
                    assets::texture::policy::TexturePolicy::for_device(
                        limits.max_texture_dimension_2d,
                    ),
                    limits.max_texture_array_layers as usize,
                )
            },
        );
        cache.finish_frame(policy, max_array_layers);
        scene.publish(
            cache.revision,
            Arc::clone(&cache.shared),
            instances,
            lines,
            DAYLIGHT,
        );
        scene.set_glint(glint.0, glint.1.scroll());
        scene.publish_terrain_instances(stream.authority().actor_session_id(), terrain_instances);
        admission.instance_limit |= placements.len() > render_model::MAX_DROPPED_ITEM_INSTANCES
            || instances.len().saturating_add(terrain_instances.len())
                > render_model::MAX_DROPPED_ITEM_INSTANCES
            || instances.len() > render_model::MAX_DROPPED_ITEM_INSTANCES
            || terrain_instances.len() > render_model::MAX_DROPPED_ITEM_INSTANCES;
        dropped.clear();
        candidates.clear();
        ropes.clear();
        admission
    }
}
