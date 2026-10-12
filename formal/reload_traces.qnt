// -*- mode: Bluespec; -*-
/**
 * The named reload scenarios as scripted step relations, so `quint run --mbt` can export
 * each one as an ITF trace with the action and picks of every step (FORMAL-07, the pattern of
 * `lifecycle_traces.qnt`, ADR 0056). `quint test --out-itf` writes the runs of
 * `scenarios.qnt` without `mbt::actionTaken` or `mbt::nondetPicks`, which the replay needs; a
 * script here takes exactly the steps of the `run` of the same name in `reloadScenarios`, one
 * per `pc`, and nothing else is enabled. The wrappers below record their fixed arguments as
 * the picks `step` records (`b`, `kinds`, `reason`, `applied`), under the model's action
 * names. A script that drifts from its run is caught by the replay only where the real
 * server disagrees, so a change to a run changes its script in the same commit. The
 * trailing `send(B).fail()` of `coalescedCssThenJsFull` is a check, not a step, and has no
 * script step.
 *
 * Each trace is regenerated into `formal/traces/regression/` with (from the repository root;
 * the scripts are deterministic, the seed is recorded for byte-for-byte freshness, FORMAL-09):
 *
 *   npx --prefix formal --no -- quint run --backend typescript --main reloadTraces \
 *     --step <script> --max-steps <its steps> --mbt --seed 0x5ce1 \
 *     --invariant allInvariants --out-itf formal/traces/regression/reload-<script>.itf.json \
 *     formal/reload_traces.qnt
 *
 * `--max-steps` is the script's length (the comment above each one); a longer run would end
 * in a deadlock once the script is spent.
 */
module reloadTraces {
  import reload as R from "./reload"

  /// The `allInvariants` of the model, checked on every exported state.
  val allInvariants = R::allInvariants

  /// The browser every script uses (`reloadScenarios::B`).
  pure val B = "b1"

  /// The script's position: the step it takes next.
  var pc: int

  action init = all { R::init, pc' = 0 }

  // The model's actions with their arguments recorded as `step`'s picks.

  action publish(k: Set[R::Kind], r: bool): bool = {
    nondet kinds = Set(k).oneOf()
    nondet reason = Set(r).oneOf()
    R::publishWith(kinds, reason)
  }

  action connect(name: str): bool = {
    nondet b = Set(name).oneOf()
    R::connect(b)
  }

  action disconnect(name: str): bool = {
    nondet b = Set(name).oneOf()
    R::disconnect(b)
  }

  action send(name: str): bool = {
    nondet b = Set(name).oneOf()
    R::send(b)
  }

  action navigate(name: str): bool = {
    nondet b = Set(name).oneOf()
    R::navigate(b)
  }

  action receive(name: str, worked: bool): bool = {
    nondet b = Set(name).oneOf()
    nondet applied = Set(worked).oneOf()
    R::receiveWith(b, applied)
  }

  /// Takes `a` as the script's step `k`.
  action at(k: int, a: bool): bool = all { pc == k, pc' = k + 1, a }

  /// `reloadScenarios::editDuringFetch`: 7 steps.
  action editDuringFetch = any {
    at(0, navigate(B)),
    at(1, publish(Set(R::Css), false)),
    at(2, connect(B)),
    at(3, receive(B, true)),
    at(4, navigate(B)),
    at(5, connect(B)),
    at(6, receive(B, true)),
  }

  /// `reloadScenarios::disconnectEditReconnect`: 13 steps.
  action disconnectEditReconnect = any {
    at(0, navigate(B)),
    at(1, connect(B)),
    at(2, receive(B, true)),
    at(3, disconnect(B)),
    at(4, connect(B)),
    at(5, receive(B, true)),
    at(6, disconnect(B)),
    at(7, publish(Set(R::Css), false)),
    at(8, connect(B)),
    at(9, receive(B, true)),
    at(10, navigate(B)),
    at(11, connect(B)),
    at(12, receive(B, true)),
  }

  /// `reloadScenarios::restartSamePort`: 10 steps.
  action restartSamePort = any {
    at(0, navigate(B)),
    at(1, connect(B)),
    at(2, receive(B, true)),
    at(3, R::stopServer),
    at(4, R::startServer),
    at(5, connect(B)),
    at(6, receive(B, true)),
    at(7, navigate(B)),
    at(8, connect(B)),
    at(9, receive(B, true)),
  }

  /// `reloadScenarios::cssOnlyHot`: 6 steps.
  action cssOnlyHot = any {
    at(0, navigate(B)),
    at(1, connect(B)),
    at(2, receive(B, true)),
    at(3, publish(Set(R::Css), false)),
    at(4, send(B)),
    at(5, receive(B, true)),
  }

  /// `reloadScenarios::coalescedCssThenJsFull`: 7 steps.
  action coalescedCssThenJsFull = any {
    at(0, navigate(B)),
    at(1, connect(B)),
    at(2, receive(B, true)),
    at(3, publish(Set(R::Css), false)),
    at(4, publish(Set(R::Js), false)),
    at(5, send(B)),
    at(6, receive(B, true)),
  }

  /// `reloadScenarios::htmlFull`: 6 steps.
  action htmlFull = any {
    at(0, navigate(B)),
    at(1, connect(B)),
    at(2, receive(B, true)),
    at(3, publish(Set(R::Html, R::Css), false)),
    at(4, send(B)),
    at(5, receive(B, true)),
  }

  /// `reloadScenarios::scriptNotAcceptedReloads`: 9 steps.
  action scriptNotAcceptedReloads = any {
    at(0, navigate(B)),
    at(1, connect(B)),
    at(2, receive(B, true)),
    at(3, publish(Set(R::Js), false)),
    at(4, send(B)),
    at(5, receive(B, true)),
    at(6, publish(Set(R::Js), false)),
    at(7, send(B)),
    at(8, receive(B, false)),
  }
}
