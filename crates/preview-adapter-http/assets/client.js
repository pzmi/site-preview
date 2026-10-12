// site-preview live-reload client. Served at /__site_preview/client.js and injected into HTML
// responses only; the source files on disk are never modified. In-memory state only: no
// cookies, no web storage, no browser identifier.
//
// Hot reload (ADR 0030, ADR 0031): a `reload` event that is not `full`, comes from the same
// server instance and carries exactly the next revision is applied in place when every change
// can be: stylesheets are swapped (the new link loads before the old one goes). A script change
// reloads the page and restores the scroll position, carried in `history.state.__sitePreview`
// (never storage or cookies, never the URL). Anything else, including a failed load, reloads.
(function () {
  'use strict';

  var MARK = '__sp';
  var LOAD_TIMEOUT_MS = 5000;
  var IMPORT_DEPTH = 8;
  var SCROLL_KEY = '__sitePreview';

  function normalize(state) {
    if (!state || typeof state.instance_id !== 'string') return null;
    if (typeof state.revision !== 'number' && typeof state.revision !== 'string') return null;
    return { instance: state.instance_id, revision: String(state.revision) };
  }

  function differs(state, baseline) {
    return state.instance !== baseline.instance || state.revision !== baseline.revision;
  }

  function isNext(current, baseline) {
    var before = Number(baseline.revision);
    var after = Number(current.revision);
    return Number.isSafeInteger(before) && Number.isSafeInteger(after) && after === before + 1;
  }

  // Pure core of plan 7.3 extended by ADR 0030. `baseline` is {instance, revision} or null,
  // `kind` is 'state' or 'reload', `state` is the parsed SSE payload ({instance_id, revision,
  // changes?, full?}), `reloading` is whether a reload is already under way. Returns 'reload',
  // 'hot' (try the change set in place) or 'none'. A hot apply needs exactly the next revision
  // of the same instance: across any gap the page may have missed a change.
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
    if (!differs(current, baseline)) return 'none';
    if (kind === 'reload' && state.changes.length > 0 &&
        current.instance === baseline.instance && isNext(current, baseline)) {
      return 'hot';
    }
    return 'reload';
  }

  // A state event with no baseline adopts the server's state (old injected pages).
  function adopt(baseline, kind, state) {
    if ((baseline === null || baseline === undefined) && kind === 'state') {
      return normalize(state);
    }
    return baseline;
  }

  // How a hot change set is applied: 'css' when every change is a stylesheet, 'js' when a
  // script changed and the rest are stylesheets (the reload subsumes them), else 'full'.
  function strategy(changes) {
    if (!Array.isArray(changes) || changes.length === 0) return 'full';
    var script = false;
    for (var i = 0; i < changes.length; i++) {
      var change = changes[i];
      if (!change || typeof change.path !== 'string' || change.path.charAt(0) !== '/') {
        return 'full';
      }
      if (change.kind === 'js') script = true;
      else if (change.kind !== 'css') return 'full';
    }
    return script ? 'js' : 'css';
  }

  // A history state the scroll position may be merged into: null or a plain object. Checked
  // by tag, not prototype, because a structured clone may come from another realm.
  function mergeable(state) {
    if (state === null || state === undefined) return true;
    return typeof state === 'object' && !Array.isArray(state) &&
      Object.prototype.toString.call(state) === '[object Object]';
  }

  // `state` with the scroll position added under `__sitePreview`, or null when the site keeps
  // a non-object state there (then the reload goes without restore, ADR 0031).
  function markScroll(state, scrollX, scrollY, revision) {
    if (!mergeable(state)) return null;
    var marked = {};
    if (state !== null && state !== undefined) {
      Object.keys(state).forEach(function (key) { marked[key] = state[key]; });
    }
    var mark = { scrollX: scrollX, scrollY: scrollY, revision: String(revision) };
    if (state === null || state === undefined) mark.empty = true;
    marked[SCROLL_KEY] = mark;
    return marked;
  }

  // The scroll position stored by markScroll and the state without it ({scroll, state}), or
  // null when there is none. A state that was null before is null again.
  function takeScroll(state) {
    if (!mergeable(state) || state === null || state === undefined) return null;
    var mark = state[SCROLL_KEY];
    if (!mark || typeof mark.scrollX !== 'number' || typeof mark.scrollY !== 'number') {
      return null;
    }
    var rest = {};
    Object.keys(state).forEach(function (key) {
      if (key !== SCROLL_KEY) rest[key] = state[key];
    });
    if (mark.empty === true && Object.keys(rest).length === 0) rest = null;
    return { scroll: { x: mark.scrollX, y: mark.scrollY }, state: rest };
  }

  // Path components, percent-decoded one by one, or null when one does not decode. Server
  // paths and browser URLs encode different character sets (ADR 0031), so both sides are
  // compared decoded; per component, so an encoded slash never matches a separator.
  function components(pathname) {
    var parts = pathname.split('/');
    try {
      for (var i = 0; i < parts.length; i++) parts[i] = decodeURIComponent(parts[i]);
    } catch (_) {
      return null;
    }
    return parts;
  }

  function samePath(left, right) {
    var a = components(left);
    var b = components(right);
    if (a === null || b === null || a.length !== b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (a[i] !== b[i]) return false;
    }
    return true;
  }

  function resolve(href, base) {
    try {
      return new URL(href, base);
    } catch (_) {
      return null;
    }
  }

  // Whether `href`, resolved against `base`, is a URL on `origin` (default: the base's origin)
  // whose path is the server's changed `path`. Query and fragment are ignored.
  function matchStylesheetHref(href, path, base, origin) {
    var url = typeof href === 'string' && href !== '' ? resolve(href, base) : null;
    if (url === null) return false;
    var home = origin ? null : resolve(base);
    var expected = origin || (home === null ? null : home.origin);
    return url.origin === expected && samePath(url.pathname, path);
  }

  // `href` with `__sp=<revision>` replacing any earlier one; other query parameters and the
  // fragment are kept byte for byte.
  function cacheBustUrl(href, revision, base) {
    var url = new URL(href, base);
    var kept = url.search.replace(/^\?/, '').split('&').filter(function (part) {
      return part !== '' && part !== MARK && part.indexOf(MARK + '=') !== 0;
    });
    kept.push(MARK + '=' + encodeURIComponent(String(revision)));
    url.search = '?' + kept.join('&');
    return url.href;
  }

  // Whether `sheet` imports `path` through an `@import`, at any depth. Best-effort: a sheet
  // whose rules cannot be read (cross-origin, SecurityError) imports nothing.
  function importsPath(sheet, path, origin, depth) {
    var level = depth || 0;
    if (!sheet || level > IMPORT_DEPTH) return false;
    var rules;
    try {
      rules = sheet.cssRules;
    } catch (_) {
      return false;
    }
    if (!rules) return false;
    var base = typeof sheet.href === 'string' && sheet.href !== '' ? sheet.href : origin + '/';
    for (var i = 0; i < rules.length; i++) {
      var rule = rules[i];
      if (!rule || typeof rule.href !== 'string' || !('styleSheet' in rule)) continue;
      if (matchStylesheetHref(rule.href, path, base, origin) ||
          importsPath(rule.styleSheet, path, origin, level + 1)) {
        return true;
      }
    }
    return false;
  }

  var hasDocument = typeof document !== 'undefined' && document.currentScript;
  if (!hasDocument) {
    globalThis.__sitePreviewTest = {
      decide: decide,
      adopt: adopt,
      strategy: strategy,
      matchStylesheetHref: matchStylesheetHref,
      cacheBustUrl: cacheBustUrl,
      importsPath: importsPath,
      markScroll: markScroll,
      takeScroll: takeScroll,
    };
    return;
  }

  var data = document.currentScript.dataset;
  var baseline =
    typeof data.instance === 'string' && data.instance !== '' &&
    typeof data.revision === 'string' && data.revision !== ''
      ? { instance: data.instance, revision: data.revision }
      : null;
  var reloading = false;
  var busy = false;
  var backlog = [];

  function fullReload() {
    reloading = true;
    backlog = [];
    location.reload();
  }

  // Resolves true once `element` has loaded, false on error or after the timeout.
  function loaded(element) {
    return new Promise(function (done) {
      var settled = false;
      function settle(ok) {
        if (settled) return;
        settled = true;
        done(ok);
      }
      element.addEventListener('load', function () { settle(true); });
      element.addEventListener('error', function () { settle(false); });
      setTimeout(function () { settle(false); }, LOAD_TIMEOUT_MS);
    });
  }

  // Inserts a cache-busted clone after `link` and removes `link` once the clone has loaded,
  // so the page is never unstyled.
  function swap(link, revision) {
    var clone = link.cloneNode(false);
    clone.href = cacheBustUrl(link.href, revision, document.baseURI);
    link.parentNode.insertBefore(clone, link.nextSibling);
    return loaded(clone).then(function (ok) {
      if (ok) link.remove();
      return ok;
    });
  }

  // The stylesheet links to swap for `changes`, or null when a path has none.
  function stylesheetsFor(changes) {
    var links = Array.prototype.slice.call(document.querySelectorAll('link[rel~="stylesheet"]'));
    var origin = location.origin;
    var chosen = [];
    for (var i = 0; i < changes.length; i++) {
      var found = false;
      for (var j = 0; j < links.length; j++) {
        var link = links[j];
        if (matchStylesheetHref(link.href, changes[i].path, document.baseURI, origin) ||
            importsPath(link.sheet, changes[i].path, origin, 0)) {
          found = true;
          if (chosen.indexOf(link) < 0) chosen.push(link);
        }
      }
      if (!found) return null;
    }
    return chosen;
  }

  // Records the scroll position in history.state, then reloads.
  function scriptReload(revision) {
    try {
      var history = window.history;
      var marked = markScroll(history.state, window.scrollX, window.scrollY, revision);
      if (marked !== null) history.replaceState(marked, '');
    } catch (_) {
      // A throttled or refused replaceState only loses the scroll restore.
    }
    fullReload();
  }

  // Scrolls back to the position a script reload stored, after load and once more on the
  // next frame (late layout), then removes the marker from history.state.
  function restoreScroll() {
    var history = window.history;
    var taken;
    try {
      taken = takeScroll(history.state);
    } catch (_) {
      return;
    }
    if (taken === null) return;
    function restore() {
      var current = takeScroll(history.state);
      try {
        if (current !== null) history.replaceState(current.state, '');
      } catch (_) {
        // The marker stays; a later script reload overwrites it.
      }
      window.scrollTo(taken.scroll.x, taken.scroll.y);
      window.requestAnimationFrame(function () {
        window.scrollTo(taken.scroll.x, taken.scroll.y);
      });
    }
    if (document.readyState === 'complete') restore();
    else window.addEventListener('load', restore);
  }

  // A promise of whether `state`'s change set was applied in place, or null when it cannot be.
  function apply(state, revision) {
    if (strategy(state.changes) !== 'css') return null;
    var links = stylesheetsFor(state.changes);
    if (links === null) return null;
    return Promise.all(links.map(function (link) { return swap(link, revision); }))
      .then(function (results) { return results.indexOf(false) < 0; });
  }

  function hot(state) {
    var revision = String(state.revision);
    if (strategy(state.changes) === 'js') {
      scriptReload(revision);
      return;
    }
    var applying;
    try {
      applying = apply(state, revision);
    } catch (_) {
      applying = null;
    }
    if (applying === null) {
      fullReload();
      return;
    }
    busy = true;
    applying.then(null, function () { return false; }).then(function (ok) {
      busy = false;
      if (!ok) {
        fullReload();
        return;
      }
      baseline = { instance: baseline.instance, revision: revision };
      while (!busy && !reloading && backlog.length > 0) {
        var next = backlog.shift();
        handle(next[0], next[1]);
      }
    });
  }

  // Events that arrive during a hot apply wait for it: the baseline they are compared with
  // is only known once it has finished.
  function handle(kind, event) {
    if (busy) {
      backlog.push([kind, event]);
      return;
    }
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
    var action = decide(baseline, kind, state, reloading);
    if (action === 'reload') fullReload();
    else if (action === 'hot') hot(state);
  }

  restoreScroll();
  var connection = new EventSource('/__site_preview/events');
  connection.addEventListener('state', function (event) { handle('state', event); });
  connection.addEventListener('reload', function (event) { handle('reload', event); });
})();
