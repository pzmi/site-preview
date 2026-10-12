// -*- mode: Bluespec; -*-
/**
 * The named lifecycle scenarios as scripted step relations, so `quint run --mbt` can export
 * each one as an ITF trace with the action and picks of every step (FORMAL-05, ADR 0056).
 * `quint test --out-itf` writes the runs of `scenarios.qnt` without `mbt::actionTaken` or
 * `mbt::nondetPicks`, which the replay needs; a script here takes exactly the steps of the
 * `run` of the same name in `lifecycleScenarios`, one per `pc`, and nothing else is enabled.
 * A script that drifts from its run is caught by the replay only where the real server
 * disagrees, so a change to a run changes its script in the same commit.
 *
 * Each trace is regenerated into `formal/traces/regression/` with (from the repository root;
 * the scripts are deterministic, the seed is recorded for byte-for-byte freshness, FORMAL-09):
 *
 *   npx --prefix formal --no -- quint run --backend typescript --main lifecycleTraces \
 *     --step <script> --max-steps <its steps> --mbt --seed 0x5ce1 \
 *     --invariant allInvariants --out-itf formal/traces/regression/lifecycle-<script>.itf.json \
 *     formal/lifecycle_traces.qnt
 *
 * `--max-steps` is the script's length (the comment above each one); a longer run would end
 * in a deadlock once the script is spent.
 */
module lifecycleTraces {
  import lifecycle as L from "./lifecycle"

  /// The `allInvariants` of the model, checked on every exported state.
  val allInvariants = L::allInvariants

  /// The script's position: the step it takes next.
  var pc: int

  action init = all { L::init, pc' = 0 }

  /// `L::startServerWith`, with its arguments recorded as the picks `bindOk` and `watcherOk`,
  /// so the trace names the step `startServer` as `serverStep`'s traces do.
  action startServer(bind: bool, watcher: bool): bool = {
    nondet bindOk = Set(bind).oneOf()
    nondet watcherOk = Set(watcher).oneOf()
    L::startServerWith(bindOk, watcherOk)
  }

  /// Takes `a` as the script's step `k`.
  action at(k: int, a: bool): bool = all { pc == k, pc' = k + 1, a }

  /// `lifecycleScenarios::startServeStop`: 5 steps.
  action startServeStop = any {
    at(0, startServer(true, true)),
    at(1, L::request),
    at(2, L::openStream),
    at(3, L::stopServer),
    at(4, L::request),
  }

  /// `lifecycleScenarios::bindFailure`: 3 steps.
  action bindFailure = any {
    at(0, startServer(false, true)),
    at(1, L::request),
    at(2, L::stopServer),
  }

  /// `lifecycleScenarios::watcherFailsDuringStart`: 3 steps.
  action watcherFailsDuringStart = any {
    at(0, startServer(true, false)),
    at(1, L::request),
    at(2, L::stopServer),
  }

  /// `lifecycleScenarios::doubleStop`: 4 steps.
  action doubleStop = any {
    at(0, startServer(true, true)),
    at(1, L::openStream),
    at(2, L::stopServer),
    at(3, L::stopServer),
  }

  /// `lifecycleScenarios::refusedBeforeStart`: 2 steps.
  action refusedBeforeStart = any {
    at(0, L::refuseStart),
    at(1, startServer(true, true)),
  }

  /// `lifecycleScenarios::restartAfterStop`: 3 steps.
  action restartAfterStop = any {
    at(0, startServer(true, true)),
    at(1, L::stopServer),
    at(2, startServer(true, true)),
  }
}
