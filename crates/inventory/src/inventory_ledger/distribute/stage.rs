//! Atomic drag planning retains cells and new requests, never the session catalog or history.

use super::super::cells::Cells;
use super::super::gesture::{Built, Submission};
use super::super::{InventoryGestureError, PlayerInventoryLedger};

pub(super) struct DistributionStage {
    cells: Cells,
    requests: Vec<Submission>,
    next_request_id: i32,
    pending: usize,
}

impl DistributionStage {
    /// Shares the current cell backing until a drag changes a surface.
    pub(super) fn new(ledger: &PlayerInventoryLedger) -> Self {
        Self {
            cells: ledger.view().clone(),
            requests: Vec::new(),
            next_request_id: ledger.next_request_id,
            pending: ledger.pending_requests().count(),
        }
    }

    /// Inputs for the next atomic transfer, including earlier reserved sparse identities.
    pub(super) fn cells(&self) -> &Cells {
        &self.cells
    }

    /// Validates and predicts one transfer without changing the real ledger.
    pub(super) fn push(
        &mut self,
        ledger: &PlayerInventoryLedger,
        built: Built,
        personal_generation: Option<u64>,
    ) -> Result<(), InventoryGestureError> {
        if self.pending + self.requests.len() >= super::super::MAX_PENDING_REQUESTS {
            return Err(InventoryGestureError::Busy);
        }
        if self.requests.len() >= protocol::MAX_STACK_REQUEST_ACTIONS {
            return Err(InventoryGestureError::InvalidRequest);
        }
        let next = self
            .next_request_id
            .checked_sub(2)
            .ok_or(InventoryGestureError::InvalidRequest)?;
        let submission = Submission {
            actions: vec![built.action],
            groups: vec![built.group],
            personal_generation,
            requires_distinct_stack_ids: built.requires_distinct_stack_ids,
            registry_bound_merge: built.registry_bound_merge,
        };
        let group = &submission.groups[0];
        ledger.validate_group(&self.cells, group, &submission.actions)?;
        if ledger.authority == Some(protocol::InventoryAuthority::Client) {
            ledger.validate_legacy_submission_at(&self.cells, &submission)?;
        }
        ledger.check_surfaces(group.touched())?;
        if !group.apply(&mut self.cells) {
            return Err(InventoryGestureError::InvalidRequest);
        }
        if ledger.authority == Some(protocol::InventoryAuthority::Server) {
            for cell in group.touched() {
                if let Some(held) = self.cells.get_mut(cell) {
                    held.bind_prediction(self.next_request_id, cell);
                }
            }
        }
        self.next_request_id = next;
        self.requests.push(submission);
        Ok(())
    }

    /// Publishes preflighted transfers in their original order and individual request scopes.
    ///
    /// Transfers were validated against successive staged cells, so a refusal here is a
    /// ledger invariant break; it stops at the last admitted transfer instead of panicking.
    pub(super) fn commit(self, ledger: &mut PlayerInventoryLedger) -> Option<i32> {
        let mut request = None;
        for submission in self.requests {
            match ledger.submit_request_with_projection(submission, None, true) {
                Ok(id) => request = Some(id),
                Err(error) => {
                    debug_assert!(false, "staged drag transfer refused: {error:?}");
                    tracing::warn!(target: "cinnabar::inventory_requests", ?error,
                        "staged drag transfer refused on commit");
                    break;
                }
            }
        }
        request
    }
}
