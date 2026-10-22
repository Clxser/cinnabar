//! A pause adopts one callback-confirmed cursor; later paused statuses cannot creep across sample grids.

use recording_studio::{MediaTime, audio::PreviewStatus, playback::Transport};

#[derive(Default)]
pub(in crate::menu::studio) struct PauseSync {
    epoch: Option<[u64; 3]>,
    was_playing: bool,
    pending: bool,
    available: Option<MediaTime>,
}

impl PauseSync {
    /// Document/edit/seek changes discard pending acknowledgements from the previous audio history.
    pub(in crate::menu::studio) fn reconcile(
        &mut self,
        transport: &mut Transport,
        status: &PreviewStatus,
        duration: MediaTime,
        available: Option<MediaTime>,
    ) -> bool {
        let epoch = status
            .epoch
            .filter(|_| status.engaged && status.error.is_none());
        if self.epoch != epoch {
            *self = Self {
                epoch,
                ..Default::default()
            };
        }
        if epoch.is_none() {
            return false;
        }
        // Hidden editor routes retire their picture before the audio callback acknowledges Pause.
        // Credit already granted to this epoch still bounds that final audible cursor.
        if let Some(available) = available {
            self.available = Some(self.available.map_or(available, |old| old.max(available)));
        }
        if transport.playing() && !self.pending {
            self.was_playing = true;
            return false;
        }
        self.pending |= self.was_playing && !transport.playing();
        self.was_playing = transport.playing();
        if !self.pending {
            return false;
        }
        let (Some(position), Some(available)) = (status.paused_position, self.available) else {
            return false;
        };
        self.pending = false;
        let before = transport.position();
        if transport.playing() {
            transport.synchronize_audio(position, duration, available);
        } else {
            transport.reconcile_paused_audio(position, duration, available);
        }
        before != transport.position()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_waits_for_the_callback_then_adopts_its_cursor_once_without_seeking() {
        let duration = MediaTime(1_000_000);
        let mut transport = Transport::default();
        transport.seek(MediaTime(100_000), duration);
        transport.play(duration);
        let epoch = transport.epoch();
        let mut sync = PauseSync::default();
        let mut status = PreviewStatus {
            epoch: Some([1, 2, epoch]),
            engaged: true,
            position: MediaTime(150_000),
            ..Default::default()
        };
        let available = Some(MediaTime(200_000));
        assert!(!sync.reconcile(&mut transport, &status, duration, available));
        transport.pause();
        assert!(!sync.reconcile(&mut transport, &status, duration, available));
        assert_eq!(transport.position(), MediaTime(100_000));
        status.paused_position = Some(MediaTime(150_000));
        assert!(sync.reconcile(&mut transport, &status, duration, available));
        assert_eq!(transport.position(), MediaTime(150_000));
        assert_eq!(transport.epoch(), epoch);
        assert!(!transport.playing());
        status.paused_position = Some(MediaTime(180_000));
        assert!(!sync.reconcile(&mut transport, &status, duration, available));
        assert_eq!(transport.position(), MediaTime(150_000));
        transport.play(duration);
        assert!(!sync.reconcile(&mut transport, &status, duration, available));
        transport.pause();
        status.epoch = Some([1, 3, epoch]);
        assert!(!sync.reconcile(&mut transport, &status, duration, available));
        assert_eq!(transport.position(), MediaTime(150_000));
    }

    #[test]
    fn hidden_picture_keeps_only_its_matching_epoch_credit_until_the_pause_acknowledgement() {
        let duration = MediaTime(1_000_000);
        let mut transport = Transport::default();
        transport.seek(MediaTime(100_000), duration);
        transport.play(duration);
        let epoch = transport.epoch();
        let mut sync = PauseSync::default();
        let mut status = PreviewStatus {
            epoch: Some([1, 2, epoch]),
            engaged: true,
            position: MediaTime(110_000),
            ..Default::default()
        };
        assert!(!sync.reconcile(&mut transport, &status, duration, Some(MediaTime(120_000))));
        transport.pause();
        assert!(!sync.reconcile(&mut transport, &status, duration, None));
        assert!(!sync.reconcile(&mut transport, &status, duration, Some(MediaTime(110_000))));
        status.paused_position = Some(MediaTime(125_000));
        assert!(sync.reconcile(&mut transport, &status, duration, None));
        assert_eq!(transport.position(), MediaTime(120_000));
        assert_eq!(transport.epoch(), epoch);
        assert!(!sync.reconcile(&mut transport, &PreviewStatus::default(), duration, None));
        status.epoch = Some([1, 3, epoch]);
        assert!(!sync.reconcile(&mut transport, &status, duration, None));
        assert_eq!(transport.position(), MediaTime(120_000));
    }

    #[test]
    fn returning_and_pressing_play_before_the_pending_pause_ack_starts_after_the_audible_tail() {
        let duration = MediaTime(1_000_000);
        let mut transport = Transport::default();
        transport.seek(MediaTime(100_000), duration);
        transport.play(duration);
        let epoch = transport.epoch();
        let mut sync = PauseSync::default();
        let mut status = PreviewStatus {
            epoch: Some([1, 2, epoch]),
            engaged: true,
            ..Default::default()
        };
        assert!(!sync.reconcile(&mut transport, &status, duration, Some(MediaTime(200_000))));
        transport.pause();
        assert!(!sync.reconcile(&mut transport, &status, duration, None));
        transport.play(duration);
        status.paused_position = Some(MediaTime(150_000));
        assert!(sync.reconcile(&mut transport, &status, duration, None));
        assert_eq!(transport.position(), MediaTime(150_000));
        assert!(transport.playing());
        assert_eq!(transport.epoch(), epoch);
    }
}
