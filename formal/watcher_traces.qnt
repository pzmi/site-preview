// -*- mode: Bluespec; -*-
/**
 * The named watcher scenarios as scripted step relations, so `quint run --mbt` can export
 * each one as an ITF trace with the action and picks of every step (FORMAL-06, ADR 0058),
 * as `lifecycle_traces.qnt` does for the lifecycle (ADR 0056). A script takes exactly the
 * steps of the `run` of the same name in `watcherScenarios`, one per `pc`, and nothing else
 * is enabled; the `run`'s `expect`s stay in `scenarios.qnt`. A script that drifts from its run
 * is caught by the replay only where the real coordinator disagrees, so a change to a run
 * changes its script in the same commit.
 *
 * Each trace is regenerated into `formal/traces/regression/` with (from the repository root;
 * the scripts are deterministic, the seed is recorded for byte-for-byte freshness, FORMAL-09):
 *
 *   npx --prefix formal --no -- quint run --backend typescript --main watcherTraces \
 *     --init <init> --step <script> --max-steps <its steps> --mbt --seed 0x5ce1 \
 *     --invariant allInvariants --out-itf formal/traces/regression/watcher-<script>.itf.json \
 *     formal/watcher_traces.qnt
 *
 * `<init>` is `init` (every directory watched at start-up, `initWith(false)`),
 * `initUnwatchable` (`d` cannot be watched, `initWith(true)`) or `initCapped` (`d`'s
 * registration passed the relisting cap, `initStartup(false, true)`), and `--max-steps` is the
 * script's length: both are in the comment above each script. A longer run would end in a
 * deadlock once the script is spent.
 */
module watcherTraces {
  import watcher as W from "./watcher"

  /// The `allInvariants` of the model, checked on every exported state.
  val allInvariants = W::allInvariants

  /// The script's position: the step it takes next.
  var pc: int

  /// `W::initWith(false)`: the root and `d` watched, `Watching`.
  action init = all { W::initWith(false), pc' = 0 }

  /// `W::initWith(true)`: `d` cannot be watched, `Starting`, the failure queued.
  action initUnwatchable = all { W::initWith(true), pc' = 0 }

  /// `W::initStartup(false, true)`: `d`'s registration passed the relisting cap, `Starting`,
  /// the loss reported first.
  action initCapped = all { W::initStartup(false, true), pc' = 0 }

  /// `W::receive`, with its arguments recorded as the picks `k` and `capped`, so the trace
  /// names the step `receive` as `step`'s traces do.
  action receive(batch: int, cap: bool): bool = {
    nondet k = Set(batch).oneOf()
    nondet capped = Set(cap).oneOf()
    W::receive(k, capped)
  }

  /// `W::settle`, with its argument recorded as the pick `capped`.
  action settle(cap: bool): bool = {
    nondet capped = Set(cap).oneOf()
    W::settle(capped)
  }

  /// Takes `a` as the script's step `k`.
  action at(k: int, a: bool): bool = all { pc == k, pc' = k + 1, a }

  /// `watcherScenarios::burstOneReload`: `init`, 12 steps.
  action burstOneReload = any {
    at(0, W::modify(W::Css)),
    at(1, W::modify(W::Css)),
    at(2, W::duplicate(0)),
    at(3, receive(1, false)),
    at(4, W::tick),
    at(5, W::modify(W::Index)),
    at(6, receive(4, false)),
    at(7, W::tick),
    at(8, W::tick),
    at(9, settle(false)),
    at(10, W::tick),
    at(11, W::tick),
  }

  /// `watcherScenarios::gitOnlyNoReload`: `init`, 8 steps.
  action gitOnlyNoReload = any {
    at(0, W::gitChange(true)),
    at(1, W::gitChange(true)),
    at(2, W::tempWrite),
    at(3, W::gitChange(false)),
    at(4, receive(4, false)),
    at(5, W::tick),
    at(6, W::tick),
    at(7, W::tick),
  }

  /// `watcherScenarios::atomicSave`: `init`, 10 steps.
  action atomicSave = any {
    at(0, W::saveAtomically(W::Index)),
    at(1, receive(4, false)),
    at(2, W::tick),
    at(3, W::tick),
    at(4, settle(false)),
    at(5, W::saveAtomically(W::Css)),
    at(6, receive(4, false)),
    at(7, W::tick),
    at(8, W::tick),
    at(9, settle(false)),
  }

  /// `watcherScenarios::overflowRescan`: `init`, 19 steps.
  action overflowRescan = any {
    at(0, W::kernelQueue(true)),
    at(1, W::modify(W::Css)),
    at(2, W::kernelQueue(false)),
    at(3, W::modify(W::Index)),
    at(4, receive(4, false)),
    at(5, receive(4, false)),
    at(6, W::tick),
    at(7, W::tick),
    at(8, settle(false)),
    at(9, W::modify(W::Css)),
    at(10, W::modify(W::Css)),
    at(11, W::modify(W::Sub)),
    at(12, W::modify(W::Index)),
    at(13, W::modify(W::Css)),
    at(14, receive(4, false)),
    at(15, receive(4, false)),
    at(16, W::tick),
    at(17, W::tick),
    at(18, settle(false)),
  }

  /// `watcherScenarios::dirRecreate`: `init`, 16 steps.
  action dirRecreate = any {
    at(0, W::dirDelete),
    at(1, receive(4, false)),
    at(2, W::tick),
    at(3, W::tick),
    at(4, settle(false)),
    at(5, W::dirCreate(false)),
    at(6, W::createFile(W::Sub)),
    at(7, receive(4, false)),
    at(8, W::tick),
    at(9, W::tick),
    at(10, settle(false)),
    at(11, W::modify(W::Sub)),
    at(12, receive(4, false)),
    at(13, W::tick),
    at(14, W::tick),
    at(15, settle(false)),
  }

  /// `watcherScenarios::recoveryAfterWatchLimit`: `init`, 14 steps.
  action recoveryAfterWatchLimit = any {
    at(0, W::dirDelete),
    at(1, receive(4, false)),
    at(2, W::dirCreate(true)),
    at(3, receive(4, false)),
    at(4, W::tick),
    at(5, W::tick),
    at(6, settle(false)),
    at(7, W::createFile(W::Sub)),
    at(8, W::freeWatch),
    at(9, W::createFile(W::Css2)),
    at(10, receive(4, false)),
    at(11, W::tick),
    at(12, W::tick),
    at(13, settle(false)),
  }

  /// `watcherScenarios::recoveryAloneReloads`: `initUnwatchable`, 8 steps.
  action recoveryAloneReloads = any {
    at(0, receive(1, false)),
    at(1, W::modify(W::Sub)),
    at(2, W::freeWatch),
    at(3, W::saveAtomically(W::Css)),
    at(4, receive(4, false)),
    at(5, W::tick),
    at(6, W::tick),
    at(7, settle(false)),
  }

  /// `watcherScenarios::runtimeFailureDegrades`: `init`, 6 steps.
  action runtimeFailureDegrades = any {
    at(0, W::runtimeFailure),
    at(1, receive(1, false)),
    at(2, W::runtimeFailure),
    at(3, receive(1, false)),
    at(4, W::createFile(W::Css2)),
    at(5, receive(4, false)),
  }

  /// `watcherScenarios::startupRegistrationFailure`: `initUnwatchable`, 5 steps.
  action startupRegistrationFailure = any {
    at(0, receive(1, false)),
    at(1, W::freeWatch),
    at(2, W::kernelQueue(true)),
    at(3, W::modify(W::Index)),
    at(4, receive(1, false)),
  }

  /// `watcherScenarios::startupFailureBeforeOverflow`: `initUnwatchable`, 9 steps.
  action startupFailureBeforeOverflow = any {
    at(0, W::freeWatch),
    at(1, W::kernelQueue(true)),
    at(2, W::modify(W::Index)),
    at(3, W::kernelQueue(false)),
    at(4, receive(1, false)),
    at(5, receive(1, false)),
    at(6, W::tick),
    at(7, W::tick),
    at(8, settle(false)),
  }

  /// `watcherScenarios::startupCapDefersReadiness`: `initCapped`, 6 steps.
  action startupCapDefersReadiness = any {
    at(0, W::modify(W::Css)),
    at(1, receive(1, false)),
    at(2, receive(1, false)),
    at(3, W::tick),
    at(4, W::tick),
    at(5, settle(false)),
  }

  /// `watcherScenarios::rootDeleteRecreate`: `init`, 9 steps.
  action rootDeleteRecreate = any {
    at(0, W::rootEmpty),
    at(1, W::rootDelete),
    at(2, receive(4, false)),
    at(3, receive(4, false)),
    at(4, W::rootRecreate),
    at(5, receive(1, false)),
    at(6, W::tick),
    at(7, W::tick),
    at(8, settle(false)),
  }

  /// `watcherScenarios::rootRemovalEscalates`: `initUnwatchable`, 7 steps.
  action rootRemovalEscalates = any {
    at(0, receive(1, false)),
    at(1, W::rootEmpty),
    at(2, W::rootDelete),
    at(3, receive(4, false)),
    at(4, W::tick),
    at(5, W::tick),
    at(6, settle(false)),
  }

  /// `watcherScenarios::rootRemovedAfterEmptyReconcile`: `init`, 7 steps.
  action rootRemovedAfterEmptyReconcile = any {
    at(0, W::rootEmpty),
    at(1, receive(4, false)),
    at(2, W::rootDelete),
    at(3, receive(1, false)),
    at(4, W::tick),
    at(5, W::tick),
    at(6, settle(false)),
  }

  /// `watcherScenarios::shutdownDropsPending`: `init`, 5 steps.
  action shutdownDropsPending = any {
    at(0, W::modify(W::Css)),
    at(1, receive(1, false)),
    at(2, W::shutdown),
    at(3, W::tick),
    at(4, W::tick),
  }
}
