// site-preview live-reload client. Served at /__site_preview/client.js and injected into HTML
// responses only; the source files on disk are never modified. In-memory state only: no
// cookies, no web storage, no browser identifier.
(function () {
  'use strict';

  function normalize(state) {
    if (!state || typeof state.instance_id !== 'string') return null;
    if (typeof state.revision !== 'number' && typeof state.revision !== 'string') return null;
    return { instance: state.instance_id, revision: String(state.revision) };
  }

  function differs(state, baseline) {
    return state.instance !== baseline.instance || state.revision !== baseline.revision;
  }

  // Pure core of plan 7.3. `baseline` is {instance, revision} or null, `kind` is 'state' or
  // 'reload', `state` is the parsed SSE payload ({instance_id, revision, changes?, full?}),
  // `reloading` is whether a reload is already under way. Returns 'reload' or 'none'.
  function decide(baseline, kind, state, reloading) {
    if (reloading) return 'none';
    var current = normalize(state);
    if (current === null) return 'none';
    if (kind === 'reload' && (state.full === true || !Array.isArray(state.changes))) {
      return 'reload';
    }
    if (baseline === null || baseline === undefined) {
      return kind === 'state' ? 'none' : 'reload';
    }
    return differs(current, baseline) ? 'reload' : 'none';
  }

  // A state event with no baseline adopts the server's state (old injected pages).
  function adopt(baseline, kind, state) {
    if ((baseline === null || baseline === undefined) && kind === 'state') {
      return normalize(state);
    }
    return baseline;
  }

  var hasDocument = typeof document !== 'undefined' && document.currentScript;
  if (!hasDocument) {
    globalThis.__sitePreviewTest = { decide: decide, adopt: adopt };
    return;
  }

  var data = document.currentScript.dataset;
  var baseline =
    typeof data.instance === 'string' && data.instance !== '' &&
    typeof data.revision === 'string' && data.revision !== ''
      ? { instance: data.instance, revision: data.revision }
      : null;
  var reloading = false;

  function handle(kind, event) {
    var state;
    try {
      state = JSON.parse(event.data);
    } catch (_) {
      return;
    }
    var adopted = adopt(baseline, kind, state);
    if (adopted !== baseline) {
      baseline = adopted;
      return;
    }
    if (decide(baseline, kind, state, reloading) === 'reload') {
      reloading = true;
      location.reload();
    }
  }

  var connection = new EventSource('/__site_preview/events');
  connection.addEventListener('state', function (event) { handle('state', event); });
  connection.addEventListener('reload', function (event) { handle('reload', event); });
})();
