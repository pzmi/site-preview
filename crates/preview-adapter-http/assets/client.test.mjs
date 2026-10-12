import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const source = readFileSync(fileURLToPath(new URL('./client.js', import.meta.url)), 'utf8');
const sandbox = { URL };
vm.runInNewContext(source, sandbox);
const { decide, adopt, strategy, matchStylesheetHref, cacheBustUrl, importsPath } =
  sandbox.__sitePreviewTest;

const ORIGIN = 'http://127.0.0.1:8000';
const flush = () => new Promise((resolve) => setImmediate(resolve));

// A minimal fake page: one container of `<link>` elements, a fake EventSource, recorded
// timers and reloads. Only what client.js touches is modelled.
function fakePage({ dataset = { instance: 'a', revision: '3' } } = {}) {
  const head = {
    children: [],
    insertBefore(node, ref) {
      const at = ref ? this.children.indexOf(ref) : -1;
      this.children.splice(at < 0 ? this.children.length : at, 0, node);
      node.parentNode = this;
    },
  };
  function element(fields) {
    const node = {
      rel: 'stylesheet',
      sheet: null,
      parentNode: null,
      handlers: {},
      ...fields,
      get nextSibling() {
        return head.children[head.children.indexOf(this) + 1] ?? null;
      },
      cloneNode() {
        return element({ rel: this.rel, href: this.href });
      },
      addEventListener(name, fn) {
        (this.handlers[name] ??= []).push(fn);
      },
      fire(name) {
        (this.handlers[name] ?? []).forEach((fn) => fn({}));
      },
      remove() {
        const at = head.children.indexOf(this);
        if (at >= 0) head.children.splice(at, 1);
        this.parentNode = null;
      },
    };
    return node;
  }
  const page = {
    reloads: 0,
    timers: [],
    listeners: {},
    head,
    addLink(href, sheet = null) {
      const link = element({ href, sheet });
      head.insertBefore(link, null);
      return link;
    },
    links() {
      return head.children.filter((node) => node.rel === 'stylesheet');
    },
    send(kind, payload) {
      page.listeners[kind]({ data: JSON.stringify(payload) });
    },
  };
  page.context = {
    document: {
      currentScript: { dataset },
      baseURI: `${ORIGIN}/`,
      querySelectorAll(selector) {
        assert.equal(selector, 'link[rel~="stylesheet"]');
        return page.links();
      },
    },
    location: { origin: ORIGIN, reload() { page.reloads += 1; } },
    EventSource: function (url) {
      page.url = url;
      this.addEventListener = (name, fn) => { page.listeners[name] = fn; };
    },
    setTimeout(fn, ms) { page.timers.push({ fn, ms }); },
    URL,
  };
  page.start = () => vm.runInNewContext(source, page.context);
  return page;
}

function css(revision, ...paths) {
  return {
    instance_id: 'a',
    revision,
    changes: paths.map((path) => ({ path, kind: 'css' })),
    full: false,
  };
}

const base = { instance: 'a', revision: '3' };
const same = { instance_id: 'a', revision: 3 };

test('should_adopt_state_when_baseline_missing', () => {
  // given
  const state = { instance_id: 'a', revision: 9 };

  // when
  const adopted = adopt(null, 'state', state);
  const action = decide(null, 'state', state, false);

  // then
  assert.equal(adopted.instance, 'a');
  assert.equal(adopted.revision, '9');
  assert.equal(action, 'none');
});

test('should_reload_when_revision_differs', () => {
  // given
  const state = { instance_id: 'a', revision: 4 };

  // when / then
  assert.equal(decide(base, 'state', state, false), 'reload');
  assert.equal(decide(base, 'reload', { ...state, changes: [] }, false), 'reload');
});

test('should_reload_when_instance_differs', () => {
  // given
  const state = { instance_id: 'b', revision: 3 };

  // when / then
  assert.equal(decide(base, 'state', state, false), 'reload');
});

test('should_not_reload_again_when_already_reloading', () => {
  // given
  const state = { instance_id: 'b', revision: 99 };

  // when / then
  assert.equal(decide(base, 'state', state, true), 'none');
  assert.equal(decide(base, 'reload', { ...state, full: true }, true), 'none');
});

test('should_do_nothing_when_state_equals_baseline', () => {
  // when / then
  assert.equal(decide(base, 'state', same, false), 'none');
  assert.equal(decide(base, 'reload', { ...same, changes: [{ path: '/a.css', kind: 'css' }] }, false), 'none');
});

test('should_reload_when_reload_event_is_full_or_has_no_changes_field', () => {
  // when / then
  assert.equal(decide(base, 'reload', { ...same, full: true, changes: [] }, false), 'reload');
  assert.equal(decide(base, 'reload', same, false), 'reload');
  assert.equal(decide(null, 'reload', same, false), 'reload');
});

test('should_ignore_malformed_state_when_payload_is_invalid', () => {
  // when / then
  assert.equal(decide(base, 'state', null, false), 'none');
  assert.equal(decide(base, 'state', { revision: 4 }, false), 'none');
  assert.equal(decide(base, 'reload', { instance_id: 'a' }, false), 'none');
});

test('should_not_use_storage_or_cookies_when_script_is_read', () => {
  // when / then
  assert.doesNotMatch(source, /localStorage|sessionStorage|document\.cookie/);
});

test('should_connect_and_reload_once_when_run_with_current_script', () => {
  // given
  let reloads = 0;
  const listeners = {};
  let url;
  const context = {
    document: { currentScript: { dataset: { instance: 'a', revision: '3' } } },
    location: { reload() { reloads += 1; } },
    EventSource: function (u) {
      url = u;
      this.addEventListener = (name, fn) => { listeners[name] = fn; };
    },
    JSON,
    String,
    Array,
  };

  // when
  vm.runInNewContext(source, context);
  listeners.state({ data: '{"instance_id":"a","revision":3}' });
  listeners.reload({ data: '{"instance_id":"a","revision":4}' });
  listeners.reload({ data: '{"instance_id":"a","revision":5}' });
  listeners.state({ data: 'not json' });

  // then
  assert.equal(url, '/__site_preview/events');
  assert.equal(reloads, 1);
  assert.equal(context.__sitePreviewTest, undefined);
});

test('should_cache_bust_href_when_query_present', () => {
  // given
  const base = `${ORIGIN}/docs/`;

  // when
  const fresh = cacheBustUrl('../css/site.css?v=2&q=a%20b+c#top', 4, base);
  const again = cacheBustUrl(fresh, 5, base);
  const plain = cacheBustUrl('/css/site.css', 7, base);

  // then
  assert.equal(fresh, `${ORIGIN}/css/site.css?v=2&q=a%20b+c&__sp=4#top`);
  assert.equal(again, `${ORIGIN}/css/site.css?v=2&q=a%20b+c&__sp=5#top`);
  assert.equal(plain, `${ORIGIN}/css/site.css?__sp=7`);
});

test('should_match_link_when_href_is_relative', () => {
  // given
  const base = `${ORIGIN}/docs/index.html`;

  // when / then
  assert.equal(matchStylesheetHref('../css/site.css?__sp=1', '/css/site.css', base), true);
  assert.equal(matchStylesheetHref('site.css', '/docs/site.css', base), true);
  assert.equal(matchStylesheetHref('site.css', '/css/site.css', base), false);
  assert.equal(matchStylesheetHref('//other.example/css/site.css', '/css/site.css', base), false);
  assert.equal(matchStylesheetHref('', '/docs/index.html', base), false);
});

test('should_compare_decoded_paths_when_encodings_differ', () => {
  // given
  const base = `${ORIGIN}/`;

  // when / then
  assert.equal(matchStylesheetHref('/css/a^b.css', '/css/a%5Eb.css', base), true);
  assert.equal(matchStylesheetHref('/css/\u017c%20x.css', '/css/%C5%BC%20x.css', base), true);
  assert.equal(matchStylesheetHref('/css/a%2Fb.css', '/css/a/b.css', base), false);
  assert.equal(matchStylesheetHref('/css/%E0.css', '/css/%E0.css', base), false);
});

test('should_find_import_when_sheet_imports_path', () => {
  // given
  const nested = { href: `${ORIGIN}/css/parts/base.css`, cssRules: [{ href: 'vars.css', styleSheet: null }] };
  const sheet = {
    href: `${ORIGIN}/css/site.css`,
    cssRules: [{ href: 'parts/base.css', styleSheet: nested }, { selectorText: 'body' }],
  };
  const locked = { href: `${ORIGIN}/css/x.css`, get cssRules() { throw new Error('SecurityError'); } };

  // when / then
  assert.equal(importsPath(sheet, '/css/parts/base.css', ORIGIN, 0), true);
  assert.equal(importsPath(sheet, '/css/parts/vars.css', ORIGIN, 0), true);
  assert.equal(importsPath(sheet, '/css/other.css', ORIGIN, 0), false);
  assert.equal(importsPath(locked, '/css/x.css', ORIGIN, 0), false);
  assert.equal(importsPath(null, '/css/x.css', ORIGIN, 0), false);
});

test('should_choose_css_strategy_when_every_change_is_css', () => {
  // when / then
  assert.equal(strategy(css(4, '/a.css', '/b.css').changes), 'css');
  assert.equal(strategy([{ path: '/a.css', kind: 'css' }, { path: '/b.html', kind: 'html' }]), 'full');
  assert.equal(strategy([{ path: 'a.css', kind: 'css' }]), 'full');
  assert.equal(strategy([]), 'full');
});

test('should_hot_apply_only_when_next_revision_of_same_instance', () => {
  // when / then
  assert.equal(decide(base, 'reload', css(4, '/a.css'), false), 'hot');
  assert.equal(decide(base, 'reload', css(5, '/a.css'), false), 'reload');
  assert.equal(decide(base, 'reload', { ...css(4, '/a.css'), instance_id: 'b' }, false), 'reload');
  assert.equal(decide(base, 'reload', { ...css(4, '/a.css'), full: true }, false), 'reload');
  assert.equal(decide(null, 'reload', css(4, '/a.css'), false), 'reload');
});

test('should_advance_baseline_when_hot_applied', async () => {
  // given
  const page = fakePage();
  const old = page.addLink(`${ORIGIN}/css/site.css`);
  page.start();
  page.send('state', css(3));

  // when
  page.send('reload', css(4, '/css/site.css'));
  const during = page.links().map((link) => link.href);
  page.links()[1].fire('load');
  await flush();
  page.send('state', css(4));
  page.send('reload', css(5, '/css/site.css'));

  // then
  assert.deepEqual(during, [`${ORIGIN}/css/site.css`, `${ORIGIN}/css/site.css?__sp=4`]);
  assert.equal(old.parentNode, null);
  assert.equal(page.reloads, 0);
  assert.equal(page.links().length, 2);
  assert.equal(page.links()[1].href, `${ORIGIN}/css/site.css?__sp=5`);
});

test('should_request_full_reload_when_no_stylesheet_matches', () => {
  // given
  const page = fakePage();
  page.addLink(`${ORIGIN}/css/site.css`);
  page.start();

  // when
  page.send('reload', css(4, '/css/site.css', '/css/print.css'));

  // then
  assert.equal(page.reloads, 1);
  assert.equal(page.links().length, 1);
});

test('should_swap_owning_link_when_imported_stylesheet_changes', async () => {
  // given
  const page = fakePage();
  const sheet = { href: `${ORIGIN}/css/site.css`, cssRules: [{ href: 'base.css', styleSheet: null }] };
  const owner = page.addLink(`${ORIGIN}/css/site.css`, sheet);
  page.addLink(`${ORIGIN}/css/other.css`);
  page.start();

  // when
  page.send('reload', css(4, '/css/base.css'));
  page.links()[1].fire('load');
  await flush();

  // then
  assert.equal(owner.parentNode, null);
  assert.deepEqual(page.links().map((link) => link.href), [
    `${ORIGIN}/css/site.css?__sp=4`,
    `${ORIGIN}/css/other.css`,
  ]);
  assert.equal(page.reloads, 0);
});

test('should_full_reload_when_swapped_stylesheet_fails_to_load', async () => {
  // given
  const page = fakePage();
  const old = page.addLink(`${ORIGIN}/css/site.css`);
  page.start();

  // when
  page.send('reload', css(4, '/css/site.css'));
  page.links()[1].fire('error');
  await flush();

  // then
  assert.equal(page.reloads, 1);
  assert.notEqual(old.parentNode, null);
});

test('should_full_reload_when_swapped_stylesheet_never_loads', async () => {
  // given
  const page = fakePage();
  page.addLink(`${ORIGIN}/css/site.css`);
  page.start();
  page.send('reload', css(4, '/css/site.css'));

  // when
  page.timers.forEach((timer) => timer.fn());
  await flush();

  // then
  assert.equal(page.timers[0].ms, 5000);
  assert.equal(page.reloads, 1);
});

test('should_apply_queued_event_when_previous_swap_finishes', async () => {
  // given
  const page = fakePage();
  page.addLink(`${ORIGIN}/css/site.css`);
  page.start();
  page.send('reload', css(4, '/css/site.css'));

  // when
  page.send('reload', css(5, '/css/site.css'));
  const queued = page.links().length;
  page.links()[1].fire('load');
  await flush();

  // then
  assert.equal(queued, 2);
  assert.equal(page.reloads, 0);
  assert.deepEqual(page.links().map((link) => link.href), [
    `${ORIGIN}/css/site.css?__sp=4`,
    `${ORIGIN}/css/site.css?__sp=5`,
  ]);
});
