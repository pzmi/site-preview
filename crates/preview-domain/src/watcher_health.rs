//! Watcher health (plan.md §8.4) as an explicit, finite state machine that emits the
//! [`DomainEvent`]s the application publishes (foundation.md §6.1 item 2: "a watcher failure
//! transitions through explicit error/recovery handling").
//!
//! States: `Starting`, `Watching { dirs }` and `Degraded { reason }`. Only `Watching` claims
//! live reload ([`WatcherHealth::is_live`]); plan.md §8.4 forbids claiming it otherwise.
//!
//! Events are bounded: entering `Degraded` emits one `WatcherDegraded`, and further failures
//! while degraded are absorbed, so a flapping or repeating failure cannot produce an event
//! storm. The one exception is an escalation to [`DegradeReason::RevisionExhausted`], which is
//! reported once and is absorbing: with no revision left to publish, live reload can never
//! recover, so a later `Ready` is refused rather than reported as a recovery. A `Ready` that
//! registered no directory is refused in every state: the root is always watched, so zero
//! directories means registration did not happen, and `Watching` must never be claimed for
//! it. Pinned by `should_hold_health_invariants_when_inputs_are_random`.

use core::fmt;

use crate::events::{DegradeReason, DomainEvent};

/// The watcher's health.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WatcherHealth {
    /// Initialization has not finished.
    #[default]
    Starting,
    /// Notifications are registered on `dirs` eligible directories; live reload works.
    Watching {
        /// The number of watched directories; at least one, since the root is always
        /// watched.
        dirs: usize,
    },
    /// Live reload is unavailable; HTTP serving continues.
    Degraded {
        /// The failure that degraded it first (or the exhaustion that made it permanent).
        reason: DegradeReason,
    },
}

/// What the watcher adapter or the application reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WatcherInput {
    /// Registration (initial, or a re-registration after a reconcile or a retry) succeeded
    /// on `dirs` directories, the root included; `dirs: 0` is refused.
    Ready {
        /// The number of watched directories.
        dirs: usize,
    },
    /// The watcher failed, or a revision could not be published.
    Failed {
        /// The failure category.
        reason: DegradeReason,
    },
}

/// An accepted input: the next health and the event to publish, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthTransition {
    /// The health after the input.
    pub next: WatcherHealth,
    /// The event to publish; `None` when nothing observable changed.
    pub event: Option<DomainEvent>,
}

/// An input that is not a declared transition: `Ready` after the revisions are exhausted, or
/// `Ready` with no directory registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WatcherHealthRefusal {
    /// The health that refused.
    pub state: WatcherHealth,
    /// The refused input.
    pub input: WatcherInput,
}

impl fmt::Display for WatcherHealthRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "watcher input {:?} is not accepted in health {:?}",
            self.input, self.state
        )
    }
}

impl core::error::Error for WatcherHealthRefusal {}

impl WatcherHealth {
    /// Whether live reload is working: watching at least one directory (the root). Pinned by
    /// `should_claim_live_reload_only_when_watching`.
    #[must_use]
    pub const fn is_live(self) -> bool {
        matches!(self, Self::Watching { dirs } if dirs > 0)
    }

    /// The transition `input` causes from this health.
    ///
    /// Pinned by `should_degrade_once_when_watcher_fails_repeatedly`,
    /// `should_recover_when_ready_after_degraded`,
    /// `should_refuse_recovery_when_revisions_are_exhausted` and
    /// `should_refuse_ready_when_no_directory_is_watched`.
    ///
    /// # Errors
    /// [`WatcherHealthRefusal`] for `Ready { dirs: 0 }`, and for any `Ready` once degraded by
    /// [`DegradeReason::RevisionExhausted`].
    pub fn step(self, input: WatcherInput) -> Result<HealthTransition, WatcherHealthRefusal> {
        use DegradeReason::RevisionExhausted;
        let (next, event) = match (self, input) {
            (_, WatcherInput::Ready { dirs: 0 }) => {
                return Err(WatcherHealthRefusal { state: self, input });
            }
            (
                Self::Degraded {
                    reason: RevisionExhausted,
                },
                WatcherInput::Ready { .. },
            ) => return Err(WatcherHealthRefusal { state: self, input }),
            (Self::Degraded { .. }, WatcherInput::Ready { dirs }) => {
                (Self::Watching { dirs }, Some(DomainEvent::WatcherRecovered))
            }
            (Self::Starting | Self::Watching { .. }, WatcherInput::Ready { dirs }) => {
                (Self::Watching { dirs }, None)
            }
            (
                Self::Degraded {
                    reason: RevisionExhausted,
                },
                WatcherInput::Failed { .. },
            ) => (self, None),
            (
                Self::Degraded { .. },
                WatcherInput::Failed {
                    reason: RevisionExhausted,
                },
            ) => degrade(RevisionExhausted),
            (Self::Degraded { .. }, WatcherInput::Failed { .. }) => (self, None),
            (Self::Starting | Self::Watching { .. }, WatcherInput::Failed { reason }) => {
                degrade(reason)
            }
        };
        Ok(HealthTransition { next, event })
    }

    /// Applies `input`: on success the health moves to `next`; on refusal it is unchanged.
    ///
    /// # Errors
    /// [`WatcherHealthRefusal`] as for [`Self::step`].
    pub fn apply(&mut self, input: WatcherInput) -> Result<HealthTransition, WatcherHealthRefusal> {
        match self.step(input) {
            Ok(transition) => {
                *self = transition.next;
                Ok(transition)
            }
            Err(refusal) => Err(refusal),
        }
    }
}

fn degrade(reason: DegradeReason) -> (WatcherHealth, Option<DomainEvent>) {
    (
        WatcherHealth::Degraded { reason },
        Some(DomainEvent::WatcherDegraded { reason }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const REGISTRATION: DegradeReason = DegradeReason::RegistrationFailed;
    const RUNTIME: DegradeReason = DegradeReason::RuntimeFailure;
    const EXHAUSTED: DegradeReason = DegradeReason::RevisionExhausted;
    const NO_DIRS: WatcherInput = WatcherInput::Ready { dirs: 0 };
    const WATCHING_NOTHING: WatcherHealth = WatcherHealth::Watching { dirs: 0 };

    fn failed(reason: DegradeReason) -> WatcherInput {
        WatcherInput::Failed { reason }
    }

    #[test]
    fn should_watch_silently_when_registration_succeeds() {
        // given
        let mut health = WatcherHealth::default();
        // when
        let ready = health.apply(WatcherInput::Ready { dirs: 3 });
        // then
        assert_eq!(
            ready,
            Ok(HealthTransition {
                next: WatcherHealth::Watching { dirs: 3 },
                event: None
            })
        );
    }

    #[test]
    fn should_emit_degraded_when_watcher_fails_during_start() {
        // given
        let mut health = WatcherHealth::Starting;
        // when
        let transition = health.apply(failed(REGISTRATION));
        // then
        assert_eq!(
            transition.map(|t| t.event),
            Ok(Some(DomainEvent::WatcherDegraded {
                reason: REGISTRATION
            }))
        );
        assert_eq!(
            health,
            WatcherHealth::Degraded {
                reason: REGISTRATION
            }
        );
    }

    #[test]
    fn should_degrade_once_when_watcher_fails_repeatedly() {
        // given
        let mut health = WatcherHealth::Watching { dirs: 2 };
        // when
        let events: Vec<_> = [RUNTIME, RUNTIME, REGISTRATION, RUNTIME]
            .into_iter()
            .map(|r| health.apply(failed(r)).map(|t| t.event))
            .collect();
        // then
        assert_eq!(
            events,
            [
                Ok(Some(DomainEvent::WatcherDegraded { reason: RUNTIME })),
                Ok(None),
                Ok(None),
                Ok(None)
            ]
        );
        assert_eq!(health, WatcherHealth::Degraded { reason: RUNTIME });
    }

    #[test]
    fn should_recover_when_ready_after_degraded() {
        // given
        let mut health = WatcherHealth::Degraded { reason: RUNTIME };
        // when
        let recovered = health.apply(WatcherInput::Ready { dirs: 5 });
        let again = health.apply(WatcherInput::Ready { dirs: 6 });
        // then
        assert_eq!(
            recovered.map(|t| t.event),
            Ok(Some(DomainEvent::WatcherRecovered))
        );
        assert_eq!(again.map(|t| t.event), Ok(None));
        assert_eq!(health, WatcherHealth::Watching { dirs: 6 });
    }

    #[test]
    fn should_escalate_once_when_revisions_exhaust_while_degraded() {
        // given
        let mut health = WatcherHealth::Degraded { reason: RUNTIME };
        // when
        let escalated = health.apply(failed(EXHAUSTED));
        let repeated = health.apply(failed(EXHAUSTED));
        let other = health.apply(failed(RUNTIME));
        // then
        assert_eq!(
            escalated.map(|t| t.event),
            Ok(Some(DomainEvent::WatcherDegraded { reason: EXHAUSTED }))
        );
        assert_eq!(repeated.map(|t| t.event), Ok(None));
        assert_eq!(other.map(|t| t.event), Ok(None));
        assert_eq!(health, WatcherHealth::Degraded { reason: EXHAUSTED });
    }

    #[test]
    fn should_refuse_recovery_when_revisions_are_exhausted() {
        // given
        let mut health = WatcherHealth::Watching { dirs: 1 };
        health.apply(failed(EXHAUSTED)).expect("degrade");
        // when
        let ready = health.apply(WatcherInput::Ready { dirs: 1 });
        // then
        assert_eq!(
            ready,
            Err(WatcherHealthRefusal {
                state: WatcherHealth::Degraded { reason: EXHAUSTED },
                input: WatcherInput::Ready { dirs: 1 }
            })
        );
        assert!(ready.is_err_and(|r| r.to_string().contains("Ready")));
        assert!(!health.is_live());
    }

    #[test]
    fn should_claim_live_reload_only_when_watching() {
        // given
        let states = [
            WatcherHealth::Starting,
            WatcherHealth::Watching { dirs: 1 },
            WatcherHealth::Watching { dirs: 0 },
            WatcherHealth::Degraded { reason: RUNTIME },
            WatcherHealth::Degraded { reason: EXHAUSTED },
        ];
        // when
        let live = states.map(WatcherHealth::is_live);
        // then
        assert_eq!(live, [false, true, false, false, false]);
    }

    #[test]
    fn should_refuse_ready_when_no_directory_is_watched() {
        // given
        let states = [
            WatcherHealth::Starting,
            WatcherHealth::Watching { dirs: 2 },
            WatcherHealth::Degraded { reason: RUNTIME },
        ];
        // when
        let results =
            states.map(|mut health| (health.apply(WatcherInput::Ready { dirs: 0 }), health));
        // then
        for (before, (result, after)) in states.into_iter().zip(results) {
            assert_eq!(
                result,
                Err(WatcherHealthRefusal {
                    state: before,
                    input: WatcherInput::Ready { dirs: 0 }
                })
            );
            assert_eq!(after, before);
        }
    }

    fn is_degraded(health: WatcherHealth) -> bool {
        matches!(health, WatcherHealth::Degraded { .. })
    }

    fn is_ready(input: WatcherInput) -> bool {
        matches!(input, WatcherInput::Ready { .. })
    }

    fn any_reason() -> impl Strategy<Value = DegradeReason> {
        prop_oneof![Just(REGISTRATION), Just(RUNTIME), Just(EXHAUSTED)]
    }

    fn any_input() -> impl Strategy<Value = WatcherInput> {
        prop_oneof![
            (0_usize..4).prop_map(|dirs| WatcherInput::Ready { dirs }),
            any_reason().prop_map(failed),
        ]
    }

    proptest! {
        #[test]
        fn should_hold_health_invariants_when_inputs_are_random(
            inputs in proptest::collection::vec(any_input(), 0..40)
        ) {
            // given
            let mut health = WatcherHealth::Starting;
            let (mut degraded, mut recovered) = (0_usize, 0_usize);
            for input in inputs {
                let before = health;
                // when
                let result = health.apply(input);
                // then
                let exhausted_before = before == WatcherHealth::Degraded { reason: EXHAUSTED };
                match result {
                    Err(refusal) => {
                        prop_assert!(is_ready(input));
                        prop_assert!(exhausted_before || input == NO_DIRS);
                        prop_assert_eq!(refusal.state, before);
                        prop_assert_eq!(health, before);
                    }
                    Ok(HealthTransition { next, event }) => {
                        prop_assert_eq!(health, next);
                        match event {
                            Some(DomainEvent::WatcherDegraded { reason }) => {
                                degraded += 1;
                                prop_assert_eq!(next, WatcherHealth::Degraded { reason });
                                prop_assert!(!is_degraded(before) || reason == EXHAUSTED);
                            }
                            Some(DomainEvent::WatcherRecovered) => {
                                recovered += 1;
                                prop_assert!(is_degraded(before));
                                prop_assert!(next.is_live());
                            }
                            Some(DomainEvent::RevisionPublished(_)) => {
                                prop_assert!(false, "health never publishes revisions");
                            }
                            None => prop_assert_eq!(is_degraded(next), is_degraded(before)),
                        }
                    }
                }
                if exhausted_before {
                    prop_assert_eq!(health, before);
                }
                prop_assert!(health != WATCHING_NOTHING);
                prop_assert!(degraded <= recovered + 2);
                prop_assert!(recovered <= degraded);
            }
        }
    }
}
