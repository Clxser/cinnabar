//! Shared native block preparation with an independent received-world admission policy.

use super::*;
use recording_scene::visuals::BlockCoverage;

/// Runs native geometry, animation and canvas owners without sharing mutable replay caches.
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare(
    stream: Option<&chunk_pipeline::WorldStream>,
    partial_tick: f32,
    camera: Option<(&Transform, &Projection)>,
    collisions: &PhysicsCollisionRegistries,
    eye: Vec3,
    ui: &UiRuntimeResource,
    atmosphere: &AtmosphereFrame,
    now_seconds: f64,
    delta: f32,
    font: &RuntimeFontCatalog,
    runtime: &mut BlockEntityRuntime,
    scene: &mut BlockEntityScene,
    frame: &mut render_model::BlockEntityFrame,
    placements: &mut Vec<StaticItemPlacement>,
    profiler: Option<&render_scene::RuntimeStageProfiler>,
    cave: Option<&CaveVisibilityCache>,
    received: bool,
) -> BlockCoverage {
    let mut coverage = BlockCoverage {
        source_seconds: now_seconds,
        artwork_available: scene.has_assets(),
        ..Default::default()
    };
    if !scene.has_assets() {
        coverage.received_columns = stream.map_or(0, |stream| {
            stream
                .collision_store()
                .received_columns()
                .filter(|key| key.dimension == stream.current_dimension())
                .count() as u32
        });
        coverage.pending_columns = coverage.received_columns;
        dragon_death::update_without_atlas(
            stream,
            partial_tick,
            (!received)
                .then_some(camera)
                .flatten()
                .map(|(transform, _)| transform.translation),
            scene,
            frame,
        );
        return coverage;
    }
    // Timed in the body: a span around the chain would also count waiting on actor publication.
    let _timer = profiler
        .as_deref()
        .map(|profiler| profiler.time(render_scene::RuntimeStage::BlockEntities));

    let clock = SceneClock {
        ticks: now_seconds * TICKS_PER_SECOND,
    };
    let Some(stream) = stream else {
        runtime.bind_session(None);
        placements.clear();
        *frame = scene.update(clock, &[], &[]).clone();
        return coverage;
    };

    runtime.bind_session(Some((
        stream.authority().actor_session_id(),
        stream.current_dimension(),
    )));
    runtime.bind_beam_source(
        stream.runtime_assets(),
        stream
            .authority()
            .dimension_range(stream.current_dimension()),
    );
    runtime.missing_maps.clear();
    let dimension = stream.current_dimension();
    let beam_top = stream.authority().dimension_range(dimension).map(|range| {
        (range.base_sub_chunk_y + range.sub_chunk_count as i32) * SUB_CHUNK_SIDE as i32
    });
    let store = stream.collision_store();
    let mode = stream.network_id_mode();

    let daylight = atmosphere.daylight();
    let camera_position = (!received)
        .then_some(camera)
        .flatten()
        .map(|(transform, _)| transform.translation);
    let clip_from_world = (!received)
        .then_some(camera)
        .flatten()
        .map(|(transform, projection)| {
            projection.get_clip_from_view() * transform.to_matrix().inverse()
        });
    let culling = cave
        .as_deref()
        .zip(camera_position)
        .map(|(cave, position)| {
            culling::CaveCulling::new(
                cave,
                camera_sub_chunk_key(dimension, position),
                stream.connectivity_generation(),
                |key: SubChunkKey| stream.has_sub_chunk_connectivity(key),
            )
        });
    let mut work = culling::BlockEntityWork::default();
    let mut occlusion_budget = meshing::occlusion::OcclusionBudget::default();
    let occlusion_eye = camera
        .zip(clip_from_world)
        .and_then(|((transform, _), matrix)| {
            let eye = transform.translation.to_array();
            render_model::visibility_bounds::perspective_supports_occlusion(
                matrix,
                eye,
                f64::from(meshing::occlusion::OCCLUSION_EYE_GAP_BLOCKS),
            )
            .then_some(eye)
        });

    let cracks = ui
        .block_crack_snapshot()
        .filter(|snapshot| snapshot.dimension == dimension)
        .map_or_else(Vec::new, |snapshot| {
            let assets = stream.runtime_assets();
            let shapes = &mut runtime.shapes;
            let entries = with_second_parts(&snapshot.entries, |entry| {
                second_crack_part(collisions, store, mode, dimension, entry)
            });
            crack_instances(&entries, |entry| {
                let runtime_id = entry.layers.iter().flatten().next().copied();
                let template = runtime_id.and_then(|id| {
                    assets.model_template_at(mode, id, entry.position, |position| {
                        let side = SUB_CHUNK_SIDE as i32;
                        let key = SubChunkKey::new(
                            dimension,
                            position[0].div_euclid(side),
                            position[1].div_euclid(side),
                            position[2].div_euclid(side),
                        );
                        let [x, y, z] = position.map(|value| value.rem_euclid(side) as u8);
                        store.sub_chunk(key)?.runtime_id(0, x, y, z)
                    })
                });
                shapes.shape(assets, mode, runtime_id, entry.position, template)
            })
        });

    runtime.books.begin(clock.ticks);
    let readers = book_readers(stream, eye);
    let mut submissions = std::mem::take(&mut runtime.submissions);
    submissions.clear();
    runtime.frame = runtime.frame.wrapping_add(1);
    let frame_stamp = runtime.frame;
    // Moved out so `resolve` can borrow a template while mutating the rest of the runtime.
    let mut scans = std::mem::take(&mut runtime.columns);
    let mut held = std::mem::take(placements);
    held.clear();
    runtime.lids.begin();
    let mut rescans_left = columns::MAX_COLUMN_RESCANS_PER_FRAME;
    let eye_column = [eye.x, eye.z].map(|axis| (axis / SUB_CHUNK_SIDE as f32).floor() as i32);
    let chunk_range = |center: f32| {
        ((center - SCAN_RADIUS_BLOCKS) / SUB_CHUNK_SIDE as f32).floor() as i32
            ..=((center + SCAN_RADIUS_BLOCKS) / SUB_CHUNK_SIDE as f32).floor() as i32
    };
    let mut keys: Vec<_> = if received {
        store
            .received_columns()
            .filter(|key| key.dimension == dimension)
            .collect()
    } else {
        chunk_range(eye.x)
            .flat_map(|x| chunk_range(eye.z).map(move |z| ChunkKey::new(dimension, x, z)))
            .collect()
    };
    keys.sort_unstable();
    coverage.received_columns = keys
        .iter()
        .filter(|key| store.chunk(**key).is_some())
        .count() as u32;
    let selected = runtime.rescans.select(keys.iter().copied().filter(|key| {
        (received || !columns::is_near_column(eye_column, key.x, key.z))
            && store
                .chunk(*key)
                .is_some_and(|chunk| scans.get(key).is_none_or(|scan| !scan.is_current(chunk)))
    }));
    let mut cached_columns = 0usize;
    let mut cached_nbt = 0usize;
    let mut cached_entities = 0usize;
    let mut cached_portals = 0usize;
    let prepare_zone = bevy::log::info_span!("block_entity.prepare").entered();
    'columns: for chunk_key in keys {
        let Some(chunk) = store.chunk(chunk_key) else {
            continue;
        };
        if received {
            let bytes = chunk
                .shared_block_entities()
                .values()
                .map(|nbt| nbt.bytes().len())
                .sum::<usize>();
            let entities = chunk.shared_block_entities().len();
            if cached_columns >= recording::MAX_CACHED_COLUMNS
                || cached_nbt + bytes > world::MAX_BLOCK_ENTITY_BYTES_PER_CHUNK
                || cached_entities + entities > MAX_SUBMISSIONS * 4
            {
                coverage.cache_limit = true;
                continue;
            }
            cached_nbt += bytes;
            cached_entities += entities;
            cached_columns += 1;
        }
        let previous = scans.remove(&chunk_key);
        let near = !received && columns::is_near_column(eye_column, chunk_key.x, chunk_key.z);
        let mut deferred = 0;
        let budget = if near || selected.contains(&Some(chunk_key)) {
            &mut rescans_left
        } else {
            &mut deferred
        };
        let Some(mut scan) = columns::frame_scan(
            previous,
            chunk,
            near,
            budget,
            columns::ColumnScan::is_current,
            |previous| {
                let portals = if received {
                    portals::column_cells_limited(chunk_key, chunk, MAX_SUBMISSIONS, |id| {
                        portal_kind(collisions, mode, id)
                    })
                } else {
                    portals::column_cells(chunk_key, chunk, |id| portal_kind(collisions, mode, id))
                };
                let entities =
                    columns::routed_entities(chunk, previous, |id, runtime_id, nbt, position| {
                        block_info(runtime, collisions, mode, runtime_id)
                            .zip(nbt.parse())
                            .and_then(|(info, root)| {
                                describe(
                                    id,
                                    info.backing?,
                                    &info.name,
                                    &info.state,
                                    &root,
                                    position,
                                )
                            })
                    });
                columns::ColumnScan::new(chunk, portals, entities)
            },
        ) else {
            continue;
        };
        if received && (cached_portals + scan.portals.len() > MAX_SUBMISSIONS * 4) {
            coverage.cache_limit = true;
            continue;
        }
        if scan.is_current(chunk) {
            coverage.current_columns += 1;
        }
        cached_portals += scan.portals.len();
        if received && scan.portals.len() >= MAX_SUBMISSIONS {
            coverage.cache_limit = true;
        }
        scan.seen_frame = frame_stamp;
        let before_portals = submissions.len();
        if received {
            portals::submit_for_view(&mut submissions, &scan.portals, None);
        } else {
            portals::submit(&mut submissions, &scan.portals, eye);
        }
        let portals = (submissions.len() - before_portals) as u32;
        work.candidates += portals;
        work.prepared += portals;
        let mut full = false;
        for entity in &scan.entities {
            if submissions.len() >= MAX_SUBMISSIONS {
                full = true;
                break;
            }
            let Some(template) = entity.template.as_ref() else {
                continue;
            };
            let [x, y, z] = entity.key.position();
            let center = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
            if !received && center.distance_squared(eye) > SCAN_RADIUS_BLOCKS * SCAN_RADIUS_BLOCKS {
                continue;
            }
            // Hidden entities keep animations, map requests and canvases without geometry.
            let mut hidden = !work.admit(culling.as_ref(), clip_from_world, template, [x, y, z]);
            if !hidden
                && let Some((low, high)) = culling::bounds(template, [x, y, z])
                && let Some(eye) = occlusion_eye
                && let Some(cave) = cave.as_deref()
            {
                let _zone = bevy::log::info_span!("block_entity.occlusion").entered();
                hidden =
                    stream.terrain_hides_box(&mut occlusion_budget, eye, low, high, |_, key| {
                        cave.is_visible(key) && cave.rendered.contains_key(&key)
                    });
                work.prepared -= u32::from(hidden);
            }
            let context = FrameContext {
                stream,
                collisions: collisions,
                eye,
                delta_seconds: delta,
                now_seconds,
                font,
                readers: &readers,
            };
            let kind = if matches!(template, Template::Beacon) {
                beam_top.map(|top| {
                    let model = scan.beacons.entry(entity.key).or_insert_with(|| {
                        beacon::model(
                            runtime,
                            collisions,
                            stream.runtime_assets(),
                            store,
                            dimension,
                            mode,
                            [x, y, z],
                            top,
                        )
                    });
                    BlockEntityKind::Beacon(model.clone())
                })
            } else {
                resolve(
                    template,
                    [x, y, z],
                    &context,
                    runtime,
                    scene,
                    &mut held,
                    hidden,
                )
            };
            if let Some(kind) = kind {
                // Stateless portal surfaces are admitted from the primary palette above.
                // A missing or stale NBT record must never add or remove their geometry.
                if matches!(
                    kind,
                    BlockEntityKind::EndPortal | BlockEntityKind::EndGateway
                ) {
                    continue;
                }
                let light = if let BlockEntityKind::Bed(model) = &kind {
                    bed::light(*model, [x, y, z], |position| {
                        stream.light_level_at(position)
                    })
                } else {
                    let (block_light, sky_light) = stream.light_level_at(center.to_array());
                    model_light(&kind, block_light, sky_light, daylight)
                };
                submissions.push(BlockEntitySubmission {
                    block: [x, y, z],
                    light,
                    kind,
                });
            }
        }
        scans.insert(chunk_key, scan);
        if full {
            coverage.submission_limit = true;
            break 'columns;
        }
    }
    drop(prepare_zone);
    runtime.lids.finish();
    runtime.books.finish();
    scans.retain(|_, scan| scan.seen_frame == frame_stamp);
    runtime.columns = scans;
    prune_bell_rings(&mut runtime.bell_rings, now_seconds, |position| {
        stream
            .block_event_cue(*position)
            .filter(|cue| cue.event_type == BELL_RING_EVENT_TYPE)
            .map(|cue| cue.sequence)
    });
    let before_global = submissions.len();
    crystal_beams::submit(
        &mut submissions,
        stream.authority().crystal_beams(partial_tick),
        camera_position,
        |runtime_id, owner_position| {
            let actor = stream.authority().actor(runtime_id)?;
            stream.solved_light_at(actor.brightness_sample_position(owner_position))
        },
    );
    dragon_death::submit(
        &mut submissions,
        stream.authority().dragon_death_rays(partial_tick),
        camera_position,
        |runtime_id, owner_position| {
            let actor = stream.authority().actor(runtime_id)?;
            stream.solved_light_at(actor.brightness_sample_position(owner_position))
        },
    );
    let global = (submissions.len() - before_global) as u32;
    work.candidates += global;
    work.prepared += global;
    *placements = held;
    let _zone = bevy::log::info_span!(
        "block_entity.scene_update",
        candidates = work.candidates,
        prepared = work.prepared,
        hidden_by_visibility = work.candidates - work.prepared,
        probes = meshing::occlusion::ENTITY_OCCLUSION_PROBES - occlusion_budget.remaining(),
        submitted = submissions.len()
    )
    .entered();
    *frame = scene.update(clock, &cracks, &submissions).clone();
    coverage.pending_columns = coverage
        .received_columns
        .saturating_sub(coverage.current_columns);
    coverage.submission_limit |= submissions.len() >= MAX_SUBMISSIONS;
    coverage.vertex_limit = scene.rejected_quads() != 0;
    submissions.clear();
    runtime.submissions = submissions;
    coverage
}
