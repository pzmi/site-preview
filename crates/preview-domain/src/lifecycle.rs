//! The server lifecycle of plan.md §9 as an explicit, finite state machine (foundation.md §6.1
//! item 1).
//!
//! The composition layer performs the steps; this machine says which step may happen next and
//! which resources each transition acquires or must release, as [`LifecycleEffect`]s. Every
//! state and every input is enumerable ([`LifecycleState::ALL`], [`LifecycleInput::ALL`]), so
//! the `lifecycle.qnt` model can be retrofitted onto it one transition at a time (ADR 0014).
//!
//! Startup runs `Start` (bind) → `Bound` (start the watcher) → `WatcherReady` or
//! `WatcherFailed` → `Serving`. A watcher failure during startup still serves, degraded
//! (plan.md §8.4). A bind failure is `Failed`, which is terminal. `Stop` is accepted in every
//! state and is a no-op once the server is stopping, stopped or failed. A `Stop` during startup
//! waits for the step in flight: its late result is accepted only to release what it acquired.
//!
//! Three resources are tracked: the listener (acquired by `Bound`), the watcher (acquired by
//! `WatcherReady`) and the owned background tasks (the watcher's debounce task, spawned with
//! the watcher, and the serve task, spawned on entering `Serving`). SSE streams exist only
//! while `Serving`, since no request is answered before, so only a stop from `Serving`
//! closes them; a stop during startup joins the tasks a late `WatcherReady` brings.
//!
//! Invariants, pinned by `should_hold_lifecycle_invariants_when_inputs_are_random`:
//! every acquired listener, watcher and task is released by an effect, so they are held only
//! in the states [`LifecycleState::holds_listener`], [`LifecycleState::holds_watcher`] and
//! [`LifecycleState::holds_tasks`] name (never in `Stopped` or `Failed`); `CloseStreams` is
//! emitted exactly when a serving server stops; only `Serving` accepts requests; a refused
//! input leaves the state unchanged; `Failed` is never left.

use core::fmt;

/// Where startup is: binding the listener, or starting the watcher with the listener bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StartPhase {
    /// The listener is being bound; nothing is held yet.
    Binding,
    /// The listener is bound; the watcher is being initialized.
    StartingWatcher,
}

/// A startup step still in flight when `Stop` arrived; its result must be awaited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InFlight {
    /// The bind: a late `Bound` must close the listener it produced.
    Bind,
    /// The watcher initialization: a late `WatcherReady` must stop the watcher and join the
    /// debounce task it spawned.
    Watcher,
}

/// A lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LifecycleState {
    /// Not running; the initial state, and the state after a clean stop.
    #[default]
    Stopped,
    /// Startup in progress.
    Starting(StartPhase),
    /// Ready and serving requests; `watcher` is false when live reload is degraded because the
    /// watcher failed to start, so there is no watcher to stop.
    Serving {
        /// A watcher is running and owned.
        watcher: bool,
    },
    /// Shutting down; `awaiting` is a startup step whose result has not arrived yet.
    Stopping {
        /// The step in flight, if any.
        awaiting: Option<InFlight>,
    },
    /// Startup failed and everything acquired was released. Terminal.
    Failed,
}

impl LifecycleState {
    /// Every state, for exhaustive tests and the formal model.
    pub const ALL: [Self; 9] = [
        Self::Stopped,
        Self::Starting(StartPhase::Binding),
        Self::Starting(StartPhase::StartingWatcher),
        Self::Serving { watcher: true },
        Self::Serving { watcher: false },
        Self::Stopping { awaiting: None },
        Self::Stopping {
            awaiting: Some(InFlight::Bind),
        },
        Self::Stopping {
            awaiting: Some(InFlight::Watcher),
        },
        Self::Failed,
    ];

    /// Only a serving server answers requests ("no responses in stopped state").
    #[must_use]
    pub const fn accepts_requests(self) -> bool {
        matches!(self, Self::Serving { .. })
    }

    /// Whether a bound listener is owned in this state.
    #[must_use]
    pub const fn holds_listener(self) -> bool {
        matches!(
            self,
            Self::Starting(StartPhase::StartingWatcher) | Self::Serving { .. }
        )
    }

    /// Whether a running watcher is owned in this state.
    #[must_use]
    pub const fn holds_watcher(self) -> bool {
        matches!(self, Self::Serving { watcher: true })
    }

    /// Whether owned background tasks (the serve task, and the watcher's debounce task when
    /// there is a watcher) must still be joined in this state.
    #[must_use]
    pub const fn holds_tasks(self) -> bool {
        matches!(self, Self::Serving { .. })
    }

    /// The transition `input` causes from this state.
    ///
    /// Pinned by `should_follow_declared_table_when_every_state_meets_every_input`.
    ///
    /// # Errors
    /// [`LifecycleRefusal`] when `input` is not a declared transition from this state.
    pub const fn step(self, input: LifecycleInput) -> Result<Transition, LifecycleRefusal> {
        use LifecycleEffect as E;
        use LifecycleInput as I;
        let (next, effects): (Self, &'static [LifecycleEffect]) = match (self, input) {
            (Self::Stopped, I::Start) => (Self::Starting(StartPhase::Binding), &[E::BindListener]),
            (Self::Starting(StartPhase::Binding), I::Bound) => (
                Self::Starting(StartPhase::StartingWatcher),
                &[E::StartWatcher],
            ),
            (Self::Starting(StartPhase::Binding), I::BindFailed) => {
                (Self::Failed, &[E::AbortStartup])
            }
            (Self::Starting(StartPhase::Binding), I::Stop) => (
                Self::Stopping {
                    awaiting: Some(InFlight::Bind),
                },
                &[],
            ),
            (Self::Starting(StartPhase::StartingWatcher), I::WatcherReady) => {
                (Self::Serving { watcher: true }, &[E::AnnounceReady])
            }
            (Self::Starting(StartPhase::StartingWatcher), I::WatcherFailed) => (
                Self::Serving { watcher: false },
                &[E::Degraded, E::AnnounceReady],
            ),
            (Self::Starting(StartPhase::StartingWatcher), I::Stop) => (
                Self::Stopping {
                    awaiting: Some(InFlight::Watcher),
                },
                &[E::CloseListener],
            ),
            (Self::Serving { watcher: true }, I::Stop) => (
                Self::Stopping { awaiting: None },
                &[
                    E::CloseListener,
                    E::CloseStreams,
                    E::StopWatcher,
                    E::JoinTasks,
                ],
            ),
            (Self::Serving { watcher: false }, I::Stop) => (
                Self::Stopping { awaiting: None },
                &[E::CloseListener, E::CloseStreams, E::JoinTasks],
            ),
            (
                Self::Stopping {
                    awaiting: Some(InFlight::Bind),
                },
                I::Bound,
            ) => (Self::Stopping { awaiting: None }, &[E::CloseListener]),
            (
                Self::Stopping {
                    awaiting: Some(InFlight::Bind),
                },
                I::BindFailed,
            )
            | (
                Self::Stopping {
                    awaiting: Some(InFlight::Watcher),
                },
                I::WatcherFailed,
            ) => (Self::Stopping { awaiting: None }, &[]),
            (
                Self::Stopping {
                    awaiting: Some(InFlight::Watcher),
                },
                I::WatcherReady,
            ) => (
                Self::Stopping { awaiting: None },
                &[E::StopWatcher, E::JoinTasks],
            ),
            (Self::Stopping { awaiting: None }, I::Stopped) => (Self::Stopped, &[]),
            (Self::Stopped | Self::Stopping { .. } | Self::Failed, I::Stop) => (self, &[]),
            (state, input) => return Err(LifecycleRefusal { state, input }),
        };
        Ok(Transition { next, effects })
    }
}

/// Something that happened to the server, reported by the composition layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LifecycleInput {
    /// Begin startup.
    Start,
    /// The listener is bound.
    Bound,
    /// Binding the listener failed.
    BindFailed,
    /// The watcher is initialized.
    WatcherReady,
    /// The watcher could not be initialized.
    WatcherFailed,
    /// Shut down (a signal or a caller's `stop()`).
    Stop,
    /// Every shutdown effect has completed.
    Stopped,
}

impl LifecycleInput {
    /// Every input, for exhaustive tests and the formal model.
    pub const ALL: [Self; 7] = [
        Self::Start,
        Self::Bound,
        Self::BindFailed,
        Self::WatcherReady,
        Self::WatcherFailed,
        Self::Stop,
        Self::Stopped,
    ];
}

/// What the composition layer must do after a transition, in the listed order.
///
/// Shutdown effects follow plan.md §9: listener, then SSE streams, then watcher, then tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LifecycleEffect {
    /// Bind the TCP listener.
    BindListener,
    /// Initialize the filesystem watcher.
    StartWatcher,
    /// Report readiness (the `ready` line or the human startup report).
    AnnounceReady,
    /// Report that live reload is unavailable while HTTP keeps serving.
    Degraded,
    /// Release everything acquired during the failed startup and report the failure.
    AbortStartup,
    /// Stop accepting connections and release the listener.
    CloseListener,
    /// End open SSE streams.
    CloseStreams,
    /// Stop the watcher.
    StopWatcher,
    /// Join the remaining owned tasks (the serve task and the watcher's debounce task).
    JoinTasks,
}

/// An accepted input: the next state and the effects to perform, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Transition {
    /// The state after the input.
    pub next: LifecycleState,
    /// What to do, in order; empty for a no-op.
    pub effects: &'static [LifecycleEffect],
}

/// An input that is not a declared transition from the current state. The state is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LifecycleRefusal {
    /// The state that refused.
    pub state: LifecycleState,
    /// The refused input.
    pub input: LifecycleInput,
}

impl fmt::Display for LifecycleRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "lifecycle input {:?} is not accepted in state {:?}",
            self.input, self.state
        )
    }
}

impl core::error::Error for LifecycleRefusal {}

/// The lifecycle of one server, starting [`LifecycleState::Stopped`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Lifecycle {
    state: LifecycleState,
}

impl Lifecycle {
    /// A stopped server.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: LifecycleState::Stopped,
        }
    }

    /// The current state.
    #[must_use]
    pub const fn state(&self) -> LifecycleState {
        self.state
    }

    /// Applies `input`: on success the state moves to `next`; on refusal it is unchanged.
    ///
    /// # Errors
    /// [`LifecycleRefusal`] when `input` is not a declared transition.
    pub const fn apply(&mut self, input: LifecycleInput) -> Result<Transition, LifecycleRefusal> {
        match self.state.step(input) {
            Ok(transition) => {
                self.state = transition.next;
                Ok(transition)
            }
            Err(refusal) => Err(refusal),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use LifecycleEffect as E;
    use LifecycleInput as I;
    use LifecycleState as S;
    use proptest::prelude::*;

    const BINDING: S = S::Starting(StartPhase::Binding);
    const WATCHING: S = S::Starting(StartPhase::StartingWatcher);
    const DRAINING: S = S::Stopping { awaiting: None };
    const AWAIT_BIND: S = S::Stopping {
        awaiting: Some(InFlight::Bind),
    };
    const AWAIT_WATCHER: S = S::Stopping {
        awaiting: Some(InFlight::Watcher),
    };

    /// The declared transitions: every accepted (state, input) pair and its result. Anything
    /// absent here must be refused.
    const DECLARED: [(S, I, S, &[E]); 19] = [
        (S::Stopped, I::Start, BINDING, &[E::BindListener]),
        (S::Stopped, I::Stop, S::Stopped, &[]),
        (BINDING, I::Bound, WATCHING, &[E::StartWatcher]),
        (BINDING, I::BindFailed, S::Failed, &[E::AbortStartup]),
        (BINDING, I::Stop, AWAIT_BIND, &[]),
        (
            WATCHING,
            I::WatcherReady,
            S::Serving { watcher: true },
            &[E::AnnounceReady],
        ),
        (
            WATCHING,
            I::WatcherFailed,
            S::Serving { watcher: false },
            &[E::Degraded, E::AnnounceReady],
        ),
        (WATCHING, I::Stop, AWAIT_WATCHER, &[E::CloseListener]),
        (
            S::Serving { watcher: true },
            I::Stop,
            DRAINING,
            &[
                E::CloseListener,
                E::CloseStreams,
                E::StopWatcher,
                E::JoinTasks,
            ],
        ),
        (
            S::Serving { watcher: false },
            I::Stop,
            DRAINING,
            &[E::CloseListener, E::CloseStreams, E::JoinTasks],
        ),
        (AWAIT_BIND, I::Bound, DRAINING, &[E::CloseListener]),
        (AWAIT_BIND, I::BindFailed, DRAINING, &[]),
        (AWAIT_BIND, I::Stop, AWAIT_BIND, &[]),
        (
            AWAIT_WATCHER,
            I::WatcherReady,
            DRAINING,
            &[E::StopWatcher, E::JoinTasks],
        ),
        (AWAIT_WATCHER, I::WatcherFailed, DRAINING, &[]),
        (AWAIT_WATCHER, I::Stop, AWAIT_WATCHER, &[]),
        (DRAINING, I::Stopped, S::Stopped, &[]),
        (DRAINING, I::Stop, DRAINING, &[]),
        (S::Failed, I::Stop, S::Failed, &[]),
    ];

    fn declared(state: S, input: I) -> Option<Transition> {
        DECLARED
            .iter()
            .find(|(s, i, _, _)| *s == state && *i == input)
            .map(|&(_, _, next, effects)| Transition { next, effects })
    }

    fn run(inputs: &[I]) -> (Lifecycle, Vec<Result<Transition, LifecycleRefusal>>) {
        let mut lifecycle = Lifecycle::new();
        let results = inputs.iter().map(|&i| lifecycle.apply(i)).collect();
        (lifecycle, results)
    }

    #[test]
    fn should_reach_serving_degraded_when_watcher_fails_during_start() {
        // given
        let inputs = [I::Start, I::Bound, I::WatcherFailed];
        // when
        let (lifecycle, results) = run(&inputs);
        // then
        assert_eq!(lifecycle.state(), S::Serving { watcher: false });
        assert!(lifecycle.state().accepts_requests());
        assert_eq!(
            results.last().map(|r| r.map(|t| t.effects)),
            Some(Ok(&[E::Degraded, E::AnnounceReady][..]))
        );
    }

    #[test]
    fn should_be_noop_when_stop_called_twice() {
        // given
        let mut lifecycle = Lifecycle::new();
        for input in [I::Start, I::Bound, I::WatcherReady] {
            lifecycle.apply(input).expect("startup");
        }
        let first = lifecycle.apply(I::Stop).expect("first stop");
        // when
        let second = lifecycle.apply(I::Stop);
        // then
        assert_eq!(first.effects.len(), 4);
        assert_eq!(
            second,
            Ok(Transition {
                next: DRAINING,
                effects: &[]
            })
        );
        assert_eq!(lifecycle.state(), DRAINING);
        lifecycle.apply(I::Stopped).expect("drained");
        assert_eq!(
            lifecycle.apply(I::Stop),
            Ok(Transition {
                next: S::Stopped,
                effects: &[]
            })
        );
    }

    #[test]
    fn should_refuse_serving_when_bind_failed() {
        // given
        let (mut lifecycle, _) = run(&[I::Start, I::BindFailed]);
        // when
        let attempts = [
            I::Bound,
            I::WatcherReady,
            I::WatcherFailed,
            I::Start,
            I::Stopped,
        ]
        .map(|i| lifecycle.apply(i));
        // then
        assert_eq!(lifecycle.state(), S::Failed);
        assert!(!lifecycle.state().accepts_requests());
        assert!(attempts.iter().all(Result::is_err));
        assert_eq!(
            lifecycle.apply(I::Stop),
            Ok(Transition {
                next: S::Failed,
                effects: &[]
            })
        );
    }

    #[test]
    fn should_release_nothing_and_hold_nothing_when_startup_fails() {
        // given
        let mut lifecycle = Lifecycle::new();
        lifecycle.apply(I::Start).expect("start");
        // when
        let failed = lifecycle
            .apply(I::BindFailed)
            .expect("bind failure accepted");
        // then
        assert_eq!(failed.effects, &[E::AbortStartup]);
        assert!(!failed.next.holds_listener());
        assert!(!failed.next.holds_watcher());
    }

    #[test]
    fn should_close_late_listener_when_stopped_during_bind() {
        // given
        let (mut lifecycle, _) = run(&[I::Start, I::Stop]);
        // when
        let early_stopped = lifecycle.apply(I::Stopped);
        let late_bound = lifecycle.apply(I::Bound);
        // then
        assert_eq!(
            early_stopped,
            Err(LifecycleRefusal {
                state: AWAIT_BIND,
                input: I::Stopped
            })
        );
        assert_eq!(late_bound.map(|t| t.effects), Ok(&[E::CloseListener][..]));
        assert_eq!(lifecycle.apply(I::Stopped).map(|t| t.next), Ok(S::Stopped));
    }

    #[test]
    fn should_stop_late_watcher_when_stopped_during_watcher_start() {
        // given
        let (mut lifecycle, results) = run(&[I::Start, I::Bound, I::Stop]);
        // when
        let late_ready = lifecycle.apply(I::WatcherReady);
        // then
        assert_eq!(
            results.last().map(|r| r.map(|t| t.effects)),
            Some(Ok(&[E::CloseListener][..]))
        );
        assert_eq!(
            late_ready.map(|t| t.effects),
            Ok(&[E::StopWatcher, E::JoinTasks][..])
        );
        assert_eq!(lifecycle.state(), DRAINING);
    }

    #[test]
    fn should_join_tasks_only_when_tasks_are_owned() {
        // given
        let holders: Vec<_> = S::ALL.into_iter().filter(|s| s.holds_tasks()).collect();
        // when
        let stops: Vec<_> = holders.iter().map(|s| s.step(I::Stop)).collect();
        // then
        assert_eq!(
            holders,
            [S::Serving { watcher: true }, S::Serving { watcher: false }]
        );
        for stop in stops {
            let effects = stop.expect("stop accepted").effects;
            assert!(effects.contains(&E::JoinTasks) && effects.contains(&E::CloseStreams));
        }
    }

    #[test]
    fn should_restart_when_stopped_cleanly() {
        // given
        let (mut lifecycle, _) = run(&[I::Start, I::Bound, I::WatcherReady, I::Stop, I::Stopped]);
        // when
        let restart = lifecycle.apply(I::Start);
        // then
        assert_eq!(restart.map(|t| t.next), Ok(BINDING));
    }

    #[test]
    fn should_refuse_start_when_already_running() {
        // given
        let (mut lifecycle, _) = run(&[I::Start, I::Bound, I::WatcherReady]);
        // when
        let again = lifecycle.apply(I::Start);
        // then
        assert_eq!(
            again,
            Err(LifecycleRefusal {
                state: S::Serving { watcher: true },
                input: I::Start
            })
        );
        assert_eq!(lifecycle.state(), S::Serving { watcher: true });
        assert!(again.is_err_and(|r| r.to_string().contains("Start")));
    }

    #[test]
    fn should_follow_declared_table_when_every_state_meets_every_input() {
        // given
        let pairs = S::ALL
            .iter()
            .flat_map(|&s| I::ALL.iter().map(move |&i| (s, i)));
        // when
        let mismatches: Vec<_> = pairs
            .filter(|&(s, i)| s.step(i).ok() != declared(s, i))
            .collect();
        // then
        assert_eq!(mismatches, []);
    }

    #[test]
    fn should_order_shutdown_effects_when_serving_stops() {
        // given
        let serving = S::Serving { watcher: true };
        // when
        let transition = serving.step(I::Stop).expect("stop accepted");
        // then
        assert_eq!(
            transition.effects,
            &[
                E::CloseListener,
                E::CloseStreams,
                E::StopWatcher,
                E::JoinTasks
            ]
        );
    }

    fn any_input() -> impl Strategy<Value = I> {
        proptest::sample::select(I::ALL.to_vec())
    }

    proptest! {
        #[test]
        fn should_hold_lifecycle_invariants_when_inputs_are_random(
            inputs in proptest::collection::vec(any_input(), 0..40)
        ) {
            // given
            let mut lifecycle = Lifecycle::new();
            let (mut listeners, mut watchers, mut tasks) = (0_i32, 0_i32, 0_i32);
            for input in inputs {
                let before = lifecycle.state();
                // when
                let result = lifecycle.apply(input);
                // then
                match result {
                    Ok(transition) => {
                        prop_assert_eq!(Some(transition), declared(before, input));
                        prop_assert!(before != S::Failed || transition.next == S::Failed);
                        if input == I::Bound {
                            listeners += 1;
                        }
                        if input == I::WatcherReady {
                            // the watcher and its debounce task
                            watchers += 1;
                            tasks += 1;
                        }
                        if !before.accepts_requests() && transition.next.accepts_requests() {
                            // the serve task
                            tasks += 1;
                        }
                        prop_assert_eq!(
                            transition.effects.contains(&E::CloseStreams),
                            before.accepts_requests() && !transition.next.accepts_requests()
                        );
                        for effect in transition.effects {
                            match effect {
                                E::CloseListener => listeners -= 1,
                                E::StopWatcher => watchers -= 1,
                                E::JoinTasks => {
                                    prop_assert!(tasks > 0, "a join with nothing to join");
                                    tasks = 0;
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(refusal) => {
                        prop_assert_eq!(declared(before, input), None);
                        prop_assert_eq!(refusal, LifecycleRefusal { state: before, input });
                    }
                }
                let state = lifecycle.state();
                prop_assert!(S::ALL.contains(&state));
                prop_assert!((0..=1).contains(&listeners) && (0..=1).contains(&watchers));
                prop_assert_eq!(listeners == 1, state.holds_listener());
                prop_assert_eq!(watchers == 1, state.holds_watcher());
                prop_assert_eq!(tasks > 0, state.holds_tasks());
                prop_assert_eq!(state.accepts_requests(), matches!(state, S::Serving { .. }));
            }
        }
    }
}
