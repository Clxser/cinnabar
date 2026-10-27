//! Borrows gameplay observations for predicted sound presentation.
use bevy::prelude::{Local, MessageReader, Res, ResMut, Time};
use client_audio::audio::AudioEngine;
use client_audio::audio::predicted::{ConsumeAudio, LocalBlockCue, MiningAudio};
use {
    crate::{particles::ParticleInbox, runtime::world::ClientWorld},
    client_interaction::{
        movement::PhysicsCollisionRegistries, survival_mining::SurvivalMiningRuntime,
    },
    client_presentation::local_player::LocalViewPose,
};

/// Borrows current owner facts and forwards them at the existing system boundary.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drive_block_cues(
    cues: MessageReader<LocalBlockCue>,
    time: Res<Time>,
    world: Res<ClientWorld>,
    collisions: Option<Res<PhysicsCollisionRegistries>>,
    survival: Option<Res<SurvivalMiningRuntime>>,
    engine: ResMut<AudioEngine>,
    mining: Local<MiningAudio>,
    profiler: Option<Res<render_scene::RuntimeStageProfiler>>,
) {
    let _timer = profiler
        .as_deref()
        .map(|profiler| profiler.time(render_scene::RuntimeStage::Audio));
    client_audio::audio::predicted::drive_block_cues(
        cues,
        time,
        client_audio::observations::WorldObservation {
            stream: world.stream.as_ref(),
        },
        collisions
            .as_deref()
            .map(|value| value as &dyn client_audio::observations::CollisionLookup),
        survival
            .as_deref()
            .map(|value| value as &dyn client_audio::observations::MiningObservation),
        engine,
        mining,
    );
}

/// Voices the local player's attack sounds from its eye in the frame the attack is sent.
pub(crate) fn drive_attack_cues(
    cues: MessageReader<client_audio::audio::predicted::LocalAttackCue>,
    world: Res<ClientWorld>,
    view: Res<LocalViewPose>,
    engine: ResMut<AudioEngine>,
    profiler: Option<Res<render_scene::RuntimeStageProfiler>>,
) {
    let _timer = profiler
        .as_deref()
        .map(|profiler| profiler.time(render_scene::RuntimeStage::Audio));
    client_audio::audio::predicted::drive_attack_cues(
        cues,
        client_audio::observations::WorldObservation {
            stream: world.stream.as_ref(),
        },
        view.eye_translation(),
        engine,
    );
}

/// Borrows current owner facts and forwards them at the existing system boundary.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drive_consume_audio(
    item_use: Res<client_interaction::item_use::ItemUseRuntime>,
    movement: Res<client_interaction::movement::MovementTicker>,
    world: Res<ClientWorld>,
    view: Res<LocalViewPose>,
    engine: ResMut<AudioEngine>,
    state: Local<ConsumeAudio>,
    profiler: Option<Res<render_scene::RuntimeStageProfiler>>,
    eating: bevy::prelude::MessageWriter<client_audio::audio::predicted::LocalEatingCue>,
) {
    let _timer = profiler
        .as_deref()
        .map(|profiler| profiler.time(render_scene::RuntimeStage::Audio));
    let tick = movement.completed_tick();
    let admitted = world.stream.as_ref().and_then(|stream| {
        let (started_tick, duration, selection) = item_use.completed_use(tick)?;
        let authority = stream.authority();
        let profile = authority.item_use_profile(selection.item.network_id());
        Some(client_audio::audio::predicted::ConsumeUse {
            identifier: authority.item_identifier(selection.item.network_id())?,
            metadata: selection.item.metadata(),
            consumption: profile.consumption,
            started_tick,
            duration,
        })
    });
    client_audio::audio::predicted::drive_consume_audio(
        admitted,
        tick,
        view.eye_translation(),
        engine,
        state,
        eating,
    );
}

/// Borrows current owner facts and forwards them at the existing system boundary.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drive_actor_audio(
    world: Res<ClientWorld>,
    mut inbox: Option<ResMut<ParticleInbox>>,
    engine: ResMut<AudioEngine>,
    profiler: Option<Res<render_scene::RuntimeStageProfiler>>,
) {
    let _timer = profiler
        .as_deref()
        .map(|profiler| profiler.time(render_scene::RuntimeStage::Audio));
    client_audio::audio::predicted::drive_actor_audio(
        client_audio::observations::WorldObservation {
            stream: world.stream.as_ref(),
        },
        inbox
            .as_deref_mut()
            .map(|value| value as &mut dyn client_audio::observations::ParticleAudioObservation),
        engine,
    );
}
