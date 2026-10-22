//! Bounded combat-event analysis. An attempted attack never becomes a confirmed hit or kill.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::{
    MediaTime, StudioError,
    model::{Confidence, MarkerKind},
};

mod draft;
pub use draft::{AutoEditOptions, AutoEditProposal, EditStyle, suggest};

const MAX_PENDING_ATTACKS: usize = 64;
const HIT_CORRELATION_MICROS: u64 = 450_000;
const COMBO_GAP_MICROS: u64 = 1_250_000;
const KILL_CORRELATION_MICROS: u64 = 2_000_000;

/// Capture assigns a monotonic sequence before worker IO so duplicate notices cannot amplify effects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameplayEvent {
    pub sequence: u64,
    pub at: MediaTime,
    pub kind: GameplayEventKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GameplayEventKind {
    AttackIntent {
        target: u64,
    },
    Hurt {
        victim: u64,
        attacker: Option<u64>,
        player: bool,
    },
    Death {
        victim: u64,
        attacker: Option<u64>,
        player: bool,
    },
    RoundStart,
    RoundEnd,
    Bookmark {
        label: String,
    },
}

/// Retains the distinction between a status observation and server-confirmed attacker attribution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Highlight {
    pub sequence: u64,
    pub at: MediaTime,
    pub kind: MarkerKind,
    pub actor: Option<u64>,
    pub combo: u32,
    pub confidence: Confidence,
    pub label: String,
}

#[derive(Clone, Copy)]
struct PendingAttack {
    target: u64,
    at: MediaTime,
}

#[derive(Clone, Copy)]
struct HitChain {
    victim: u64,
    last: MediaTime,
    hits: u32,
    confidence: Confidence,
}

/// The capture owner creates one detector per session/dimension, resetting correlation on transfers.
pub struct CombatDetector {
    local_actor: u64,
    last_sequence: Option<u64>,
    last_time: MediaTime,
    pending: VecDeque<PendingAttack>,
    chain: Option<HitChain>,
}

impl CombatDetector {
    /// No global state or clock is consulted; recorded events produce the same highlights offline.
    pub fn new(local_actor: u64) -> Self {
        Self {
            local_actor,
            last_sequence: None,
            last_time: MediaTime::ZERO,
            pending: VecDeque::with_capacity(MAX_PENDING_ATTACKS),
            chain: None,
        }
    }

    /// Appends at most two observations per event into a caller-retained output vector.
    pub fn observe(
        &mut self,
        event: &GameplayEvent,
        out: &mut Vec<Highlight>,
    ) -> Result<(), StudioError> {
        if self
            .last_sequence
            .is_some_and(|sequence| event.sequence <= sequence)
        {
            return Ok(());
        }
        if event.at < self.last_time {
            return Err(StudioError::Invalid(
                "recorded gameplay events are out of time order",
            ));
        }
        if event.at.0 > crate::time::MAX_MEDIA_MICROS {
            return Err(StudioError::TimeOverflow);
        }
        if let GameplayEventKind::Bookmark { label } = &event.kind {
            crate::error::limit("bookmark label bytes", label.len(), 512)?;
        }
        self.last_sequence = Some(event.sequence);
        self.last_time = event.at;
        while self
            .pending
            .front()
            .is_some_and(|pending| event.at.0.saturating_sub(pending.at.0) > HIT_CORRELATION_MICROS)
        {
            self.pending.pop_front();
        }
        let make = |kind, actor, combo, confidence, label: String| Highlight {
            sequence: event.sequence,
            at: event.at,
            kind,
            actor,
            combo,
            confidence,
            label,
        };
        match &event.kind {
            GameplayEventKind::AttackIntent { target } => {
                if self.pending.len() == MAX_PENDING_ATTACKS {
                    self.pending.pop_front();
                }
                self.pending.push_back(PendingAttack {
                    target: *target,
                    at: event.at,
                });
            }
            GameplayEventKind::Hurt {
                victim,
                attacker,
                player,
            } => {
                if *victim == self.local_actor {
                    self.chain = None;
                    return Ok(());
                }
                if !player {
                    return Ok(());
                }
                let matched = self
                    .pending
                    .iter()
                    .position(|attack| attack.target == *victim);
                let confidence = match attacker {
                    Some(actor) if *actor == self.local_actor => Confidence::Confirmed,
                    Some(_) => return Ok(()),
                    None if matched.is_some() => Confidence::Correlated,
                    None => return Ok(()),
                };
                if let Some(index) = matched {
                    self.pending.remove(index);
                }
                let previous = self.chain.filter(|chain| {
                    chain.victim == *victim
                        && event.at.0.saturating_sub(chain.last.0) <= COMBO_GAP_MICROS
                });
                let hits = previous.map_or(1, |chain| chain.hits.saturating_add(1));
                let confidence =
                    if previous.is_some_and(|chain| chain.confidence == Confidence::Correlated) {
                        Confidence::Correlated
                    } else {
                        confidence
                    };
                self.chain = Some(HitChain {
                    victim: *victim,
                    last: event.at,
                    hits,
                    confidence,
                });
                out.push(make(
                    MarkerKind::Hit,
                    Some(*victim),
                    hits,
                    confidence,
                    "Hit".into(),
                ));
                if hits >= 3 {
                    out.push(make(
                        MarkerKind::Combo,
                        Some(*victim),
                        hits,
                        confidence,
                        if confidence == Confidence::Confirmed {
                            format!("{hits}-hit combo")
                        } else {
                            format!("{hits}-hit combo (estimated)")
                        },
                    ));
                }
            }
            GameplayEventKind::Death {
                victim,
                attacker,
                player,
            } => {
                if *victim == self.local_actor {
                    self.chain = None;
                    out.push(make(
                        MarkerKind::Death,
                        Some(*victim),
                        0,
                        Confidence::Observed,
                        "Death".into(),
                    ));
                    return Ok(());
                }
                if !player {
                    return Ok(());
                }
                let chain = self.chain.filter(|chain| {
                    chain.victim == *victim
                        && event.at.0.saturating_sub(chain.last.0) <= KILL_CORRELATION_MICROS
                });
                let confirmed = *attacker == Some(self.local_actor);
                let correlated = attacker.is_none() && chain.is_some();
                if confirmed || correlated {
                    out.push(make(
                        MarkerKind::Kill,
                        Some(*victim),
                        chain.map_or(0, |chain| chain.hits),
                        if confirmed {
                            Confidence::Confirmed
                        } else {
                            Confidence::Correlated
                        },
                        if confirmed {
                            "Kill".into()
                        } else {
                            "Possible kill".into()
                        },
                    ));
                } else {
                    out.push(make(
                        MarkerKind::Death,
                        Some(*victim),
                        0,
                        Confidence::Observed,
                        "Player death".into(),
                    ));
                }
                if self.chain.is_some_and(|chain| chain.victim == *victim) {
                    self.chain = None;
                }
            }
            GameplayEventKind::RoundStart | GameplayEventKind::RoundEnd => {
                self.pending.clear();
                self.chain = None;
                let (kind, label) = if matches!(event.kind, GameplayEventKind::RoundStart) {
                    (MarkerKind::RoundStart, "Round start")
                } else {
                    (MarkerKind::RoundEnd, "Round end")
                };
                out.push(make(kind, None, 0, Confidence::Observed, label.into()));
            }
            GameplayEventKind::Bookmark { label } => {
                out.push(make(
                    MarkerKind::Manual,
                    None,
                    0,
                    Confidence::Observed,
                    label.clone(),
                ));
            }
        }
        Ok(())
    }
}
