use protocol::{ActorLinkEvent, ActorLinkType, ActorMetadataValue};
use protocol_ids::ActorDataKey;

use super::{ActorApplyResult, ActorKind, ActorSnapshot};

impl ActorSnapshot {
    /// Returns the horse-family behavior cached at admission.
    pub(crate) fn is_horse(&self) -> bool {
        self.entity_fact.horse
    }

    /// Returns the living-mount disposition cached at admission.
    pub fn is_known_living_mount(&self) -> bool {
        matches!(self.kind, ActorKind::Player { .. }) || self.entity_fact.living_mount
    }

    /// The passenger index whose input steers this vehicle; 0 unless the server sets one.
    fn controlling_seat_index(&self) -> usize {
        let index = match self
            .metadata
            .get(&ActorDataKey::ControllingSeatIndex.wire_value())
        {
            Some(ActorMetadataValue::Byte(value)) => i64::from(*value),
            Some(ActorMetadataValue::Short(value)) => i64::from(*value),
            Some(ActorMetadataValue::Int(value)) => i64::from(*value),
            Some(ActorMetadataValue::Long(value)) => *value,
            _ => 0,
        };
        usize::try_from(index).unwrap_or(usize::MAX)
    }
}

impl super::ActorStore {
    /// Reads equipment capabilities cached when the actor and catalog were admitted.
    pub(crate) fn mount_equipment(&self, runtime_id: u64) -> assets::entity::mount::MountEquipment {
        self.animation.mount_equipment(runtime_id)
    }

    /// The passenger in the vehicle's controlling seat, whose input steers it.
    pub(crate) fn controlling_passenger(&self, vehicle_unique_id: i64) -> Option<i64> {
        let vehicle = self.snapshot_by_unique(vehicle_unique_id)?;
        self.passenger_order
            .get(&vehicle_unique_id)?
            .get(vehicle.controlling_seat_index())
            .copied()
    }

    fn is_player_unique(&self, unique_id: i64) -> bool {
        self.snapshot_by_unique(unique_id)
            .is_some_and(|actor| matches!(actor.kind, ActorKind::Player { .. }))
    }

    /// Applies one riding link. Link types only add or remove; seat order follows arrival,
    /// except that a player boarding a vehicle steered by a non-player takes the front seat.
    pub(super) fn apply_link_inner(&mut self, event: ActorLinkEvent) -> ActorApplyResult {
        let rider = event.rider_unique_id;
        let vehicle = event.ridden_unique_id;
        match event.link_type {
            ActorLinkType::Unknown(_) => ActorApplyResult::Updated,
            ActorLinkType::Remove => {
                if self.rider_to_ridden.get(&rider) == Some(&vehicle) {
                    self.rider_to_ridden.remove(&rider);
                    self.leave_passenger_order(rider, vehicle);
                }
                ActorApplyResult::Updated
            }
            ActorLinkType::Rider | ActorLinkType::Passenger => {
                let previous = self.rider_to_ridden.get(&rider).copied();
                if previous.is_none() && self.rider_to_ridden.len() >= self.max_actor_links {
                    return ActorApplyResult::CapacityRejected;
                }
                if previous == Some(vehicle) {
                    return ActorApplyResult::Updated;
                }
                if let Some(previous) = previous {
                    self.leave_passenger_order(rider, previous);
                }
                self.rider_to_ridden.insert(rider, vehicle);
                let rider_is_player = self.is_player_unique(rider);
                let front_is_player = self
                    .passenger_order
                    .get(&vehicle)
                    .and_then(|order| order.first())
                    .map(|front| self.is_player_unique(*front));
                let order = self.passenger_order.entry(vehicle).or_default();
                if rider_is_player && front_is_player == Some(false) {
                    order.insert(0, rider);
                } else {
                    order.push(rider);
                }
                ActorApplyResult::Updated
            }
        }
    }

    fn leave_passenger_order(&mut self, rider: i64, vehicle: i64) {
        if let Some(order) = self.passenger_order.get_mut(&vehicle) {
            order.retain(|passenger| *passenger != rider);
            if order.is_empty() {
                self.passenger_order.remove(&vehicle);
            }
        }
    }

    /// Drops every link that names this actor as rider or vehicle.
    pub(super) fn remove_links_for(&mut self, unique_id: i64) {
        if let Some(vehicle) = self.rider_to_ridden.remove(&unique_id) {
            self.leave_passenger_order(unique_id, vehicle);
        }
        self.rider_to_ridden
            .retain(|_, ridden_unique_id| *ridden_unique_id != unique_id);
        self.passenger_order.remove(&unique_id);
    }

    pub(super) fn clear_links(&mut self) {
        self.rider_to_ridden.clear();
        self.passenger_order.clear();
    }
}
