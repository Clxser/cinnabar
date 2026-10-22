//! Nonblocking immutable native terrain publications for Recording Studio's item track.

use bevy::{prelude::Resource, render::extract_resource::ExtractResource};
use render_model::dropped_item::{ItemTerrainSnapshot, MAX_ITEM_TERRAIN_PUBLICATIONS};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Default, Resource, ExtractResource)]
#[extract_app(bevy::render::RenderApp)]
pub struct ItemReplayVisibility {
    requested: Arc<AtomicBool>,
    state: Arc<Mutex<Option<ItemTerrainSnapshot>>>,
}

impl ItemReplayVisibility {
    /// Capture requests snapshots at its admitted cadence; disabled capture releases prior generations.
    pub fn request(&self, enabled: bool, due: bool) {
        if !enabled {
            self.requested.store(false, Ordering::Release);
            if let Ok(mut state) = self.state.try_lock() {
                *state = None;
            }
        } else if due {
            self.requested.store(true, Ordering::Release);
        }
    }

    /// Main-world capture only clones a completed immutable publication and never waits for rendering.
    pub fn latest(&self) -> Option<ItemTerrainSnapshot> {
        self.state.try_lock().ok()?.clone()
    }

    /// Called by the actual render visibility owner, including zero-byte and removed terrain updates.
    pub fn publish(
        &self,
        session_id: Option<u64>,
        generations: impl ExactSizeIterator<Item = (world::SubChunkKey, u64)>,
    ) {
        if !self.requested.load(Ordering::Acquire) {
            return;
        }
        let Ok(mut state) = self.state.try_lock() else {
            return;
        };
        if !self.requested.swap(false, Ordering::AcqRel) {
            return;
        }
        let complete = generations.len() <= MAX_ITEM_TERRAIN_PUBLICATIONS;
        let mut generations = generations
            .take(MAX_ITEM_TERRAIN_PUBLICATIONS)
            .collect::<Vec<_>>();
        generations.sort_unstable_by_key(|(key, _)| *key);
        let serial = state.as_ref().map_or(1, |old| old.serial.saturating_add(1));
        *state = Some(ItemTerrainSnapshot {
            serial,
            session_id,
            complete,
            generations: generations.into(),
        });
    }
}
