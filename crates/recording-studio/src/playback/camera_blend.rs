//! Entry blends use only two authored rigs and the current continuous recorded source.

use super::*;

impl CompiledTimeline {
    /// Missing targets, source cuts and ambiguous picture layers retain the incoming shot's exact fallback.
    pub(super) fn blend_camera(
        &self,
        index: usize,
        time: MediaTime,
        out: &mut FramePlan,
        actor: &impl Fn(Id, CameraTargetRef, MediaTime) -> Option<ActorPose>,
    ) {
        let Some(incoming) = out.camera else {
            return;
        };
        let Some(plan) = self.camera_blends.get(index, time) else {
            return;
        };
        let [source] = out.visuals.as_slice() else {
            return;
        };
        if source.clip != self.project.clips[plan.clip].id || source.mix.is_some() {
            return;
        }
        let shot = &self.project.shots[index];
        let previous = &self.project.shots[plan.previous];
        let Some(outgoing) = previous.sample_terminal(
            |target| actor(source.source, target, source.time),
            self.camera_paths[plan.previous].as_ref(),
        ) else {
            return;
        };
        let outgoing = plan
            .motion
            .extrapolated(outgoing, time.saturating_sub(shot.span.start));
        out.camera = Some(shot.blend_in.expect("compiled entry metadata").sample(
            outgoing,
            incoming,
            time.saturating_sub(shot.span.start),
        ));
        out.spectated = previous
            .spectated_actor()
            .filter(|target| Some(*target) == shot.spectated_actor());
        out.hide_hud |= previous.hide_hud;
        // A camera travelling between viewpoints has no matching first-person hand transform.
        out.hide_hand |= previous.hide_hand || out.spectated.is_none();
    }
}
