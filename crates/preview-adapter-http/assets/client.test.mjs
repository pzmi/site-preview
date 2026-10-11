import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const source = readFileSync(fileURLToPath(new URL('./client.js', import.meta.url)), 'utf8');
const sandbox = {};
vm.runInNewContext(source, sandbox);
const { decide, adopt } = sandbox.__sitePreviewTest;

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
