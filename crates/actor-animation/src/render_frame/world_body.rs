//! External-camera body poses use native world controllers, independent of the live hand.

use super::*;

/// Resamples both motion endpoints in world space under the frame's shared work budget.
pub(super) fn sample<'a>(
    store: &'a ActorAnimationStore,
    actor: &dyn AnimatedActor,
    (camera_rotation, camera_position, alpha): ([f32; 2], [f32; 3], f32),
    remaining_ops: &mut usize,
    sample_skin: bool,
) -> Option<ActorRenderLayers<'a>> {
    let lifetime = store.runtime_to_lifetime.get(&actor.runtime_id())?;
    let state = store.rigs.get(lifetime)?;
    if lifetime.spawn_revision != actor.spawn_revision() {
        return None;
    }
    if !state.query_context.is_local_first_person {
        return store.render_layers(
            actor,
            alpha,
            camera_rotation,
            camera_position,
            remaining_ops,
            sample_skin,
        );
    }
    let (assets, layout) = if state.pack {
        let pack = store.pack.as_ref()?;
        (pack.assets.as_ref(), pack.layout.as_ref())
    } else {
        (store.assets.as_deref()?, store.layout.as_ref())
    };
    let mut budget = EvalBudget {
        actor_left: MAX_MOLANG_OPS_PER_ACTOR_FRAME,
        world_left: remaining_ops,
        work_left: MAX_RUNTIME_POSE_WORK_PER_ACTOR_FRAME,
        transitions_left: FRAME_PASSES_PER_ACTOR * MAX_CONTROLLER_TRANSITIONS_PER_TICK,
        used: 0,
        stack: OperandStack::lease(),
        static_draw: None,
    };
    let Some(frame) = state.render_frame.as_ref() else {
        let context = world_context(
            &state.query_context,
            camera_rotation,
            camera_position,
            alpha,
        );
        let evaluated = tick::evaluate_state(
            assets,
            layout,
            state,
            actor,
            &context,
            state.completed_tick,
            &mut budget,
            false,
            None,
        )
        .ok()?;
        let pose = Arc::<[BoneTransform]>::from(evaluated.pose);
        let mut render = evaluated.render?;
        for layer in &mut render {
            if layer.geometry.is_none() {
                layer.pose = Arc::clone(&pose);
            }
            layer.previous_pose = Arc::clone(&layer.pose);
            layer.sampled_scale = evaluated.scale;
        }
        return Some(ActorRenderLayers {
            held_item_scale: evaluated.held_item_scale,
            render: Cow::Owned(render),
            skin: Cow::Owned(if sample_skin {
                evaluated.skin_layers
            } else {
                Vec::new()
            }),
        });
    };
    let previous_motion = frame.previous_motion.as_ref().unwrap_or(&frame.motion);
    let contexts = [previous_motion, &frame.motion]
        .map(|motion| world_context(&motion.context, camera_rotation, camera_position, alpha));
    // First-person endpoints deliberately zero rotations; authority history retains the body.
    let observed = *state.history.back()?;
    let previous_observed = state
        .history
        .iter()
        .rev()
        .nth(1)
        .copied()
        .unwrap_or(observed);
    let inputs = [previous_motion, &frame.motion].map(|motion| {
        motion
            .input
            .sample_rotation(previous_observed, observed, alpha)
            .sample_walk(previous_observed, observed, alpha)
    });
    let evaluator = evaluation::Evaluator {
        assets,
        pack: state.pack,
        layout,
        program: None,
        actor,
        input: &inputs[1],
        context: &contexts[1],
        anim_tick: frame.motion.anim_tick,
        anim_time: None,
        swell_amount: None,
        presentation_alpha: Some(alpha),
        query_history: None,
        life_tick: frame.motion.life_tick,
        finished: (false, false),
        state_time: 0.0,
        bones: state.posed_bones(),
        bone_names: state.posed_bone_names(),
    };
    let swing = state
        .local_swing
        .map(|swing| swing.bedrock_progress(alpha))
        .or_else(|| {
            state
                .remote_swing()
                .map(|pair| LocalSwingProgress::interpolate(pair, alpha))
        });
    let amount = actor.creeper_swell_amount(alpha);
    let mut scratch = frame.cache.lock().unwrap();
    let FrameSamplingCache {
        previous: previous_scratch,
        current: current_scratch,
        compositions,
        ..
    } = &mut *scratch;
    let mut previous = previous_motion
        .sample(
            evaluation::Evaluator {
                input: &inputs[0],
                ..evaluator
            },
            state,
            (amount, swing),
            Some(&contexts[0]),
            false,
            &mut budget,
            previous_scratch,
        )
        .ok()?;
    let mut current = frame
        .motion
        .sample(
            evaluator,
            state,
            (amount, swing),
            Some(&contexts[1]),
            false,
            &mut budget,
            current_scratch,
        )
        .ok()?;
    let previous_local = pose::sample_clips_reused(
        &previous.evaluator,
        &mut previous.variables,
        &state.bones,
        pose::ChannelBones::bound(&state.bone_names, &state.channel_bindings),
        &previous.clips,
        &mut budget,
    )
    .ok()?;
    let local = pose::sample_clips_reused(
        &current.evaluator,
        &mut current.variables,
        &state.bones,
        pose::ChannelBones::bound(&state.bone_names, &state.channel_bindings),
        &current.clips,
        &mut budget,
    )
    .ok()?;
    let body = (
        compose_body(state, &previous_local, true, compositions)?,
        compose_body(state, &local, false, compositions)?,
    );
    let render_rig = render::RenderRig {
        binding: state.rig_binding,
        geometry: assets
            .rig_geometries()
            .get(state.geometry_binding)?
            .geometry,
        bone_names: state.posed_bone_names(),
        skeletons: &state.layer_skeletons,
    };
    let mut render = render::evaluate_render(
        &current.evaluator,
        &mut current.variables,
        render_rig,
        &mut budget,
    )
    .ok()??;
    if render.iter().any(|layer| layer.geometry.is_some()) {
        // Selected geometry reads each endpoint's own render-controller assignments.
        render::evaluate_render(
            &previous.evaluator,
            &mut previous.variables,
            render_rig,
            &mut budget,
        )
        .ok()??;
    }
    let mut geometries = BTreeMap::new();
    for layer in &mut render {
        let (previous_pose, pose) = match layer.geometry {
            None => &body,
            Some(geometry) => {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    geometries.entry(geometry)
                {
                    entry.insert((
                        layer_pose(
                            state,
                            &mut previous,
                            geometry,
                            true,
                            compositions,
                            &mut budget,
                        )?,
                        layer_pose(
                            state,
                            &mut current,
                            geometry,
                            false,
                            compositions,
                            &mut budget,
                        )?,
                    ));
                }
                geometries.get(&geometry)?
            }
        };
        layer.previous_pose = Arc::clone(previous_pose);
        layer.pose = Arc::clone(pose);
        layer.sampled_scale = current.scale;
    }
    let skin = if sample_skin {
        skin_layers::sample(
            state,
            &current.evaluator,
            &current.variables,
            (&local, Some(&previous_local)),
            Some(&render),
            &mut budget,
        )
        .ok()?
    } else {
        Vec::new()
    };
    Some(ActorRenderLayers {
        held_item_scale: current.held_item_scale,
        render: Cow::Owned(render),
        skin: Cow::Owned(skin),
    })
}

/// Reuses a body endpoint's matrices when its sampled local channels have not changed.
fn compose_body(
    state: &ActorRigState,
    local: &[pose::LocalDelta],
    previous: bool,
    compositions: &mut composition::Compositions,
) -> Option<Arc<[BoneTransform]>> {
    let count = state.posed_bones().len();
    compositions.compose(None, previous, count, local, || {
        let mut model = vec![BoneTransform::default(); count];
        state.compose_into(local, &mut model)?;
        Some(model)
    })
}

/// Composes a controller-selected model on its own skeleton using the world-body clips.
fn layer_pose(
    state: &ActorRigState,
    endpoint: &mut MotionEndpoint<'_>,
    geometry: u32,
    previous: bool,
    compositions: &mut composition::Compositions,
    budget: &mut EvalBudget<'_>,
) -> Option<Arc<[BoneTransform]>> {
    let skeleton = state.layer_skeletons.get(&geometry)?.as_ref()?;
    let local = render::sample_layer_local(
        &endpoint.evaluator,
        &mut endpoint.variables,
        &state.layer_skeletons,
        &endpoint.clips,
        geometry,
        budget,
    )
    .ok()?;
    compositions.compose(
        Some(geometry),
        previous,
        skeleton.bones.len(),
        &local,
        || pose::compose_pose(&skeleton.bones, &local),
    )
}

/// Replaces only perspective and camera inputs; equipment and native animation clocks survive.
fn world_context(
    source: &ActorTickContext,
    camera_rotation: [f32; 2],
    camera_position: [f32; 3],
    alpha: f32,
) -> ActorTickContext {
    ActorTickContext {
        is_local_first_person: false,
        is_in_ui: false,
        frame_alpha: alpha,
        camera_rotation,
        camera_position,
        ..source.clone()
    }
}
