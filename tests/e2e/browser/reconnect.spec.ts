// Browser reconnect and restart recovery (E2E-06, plan section 12 browser steps 6-7, plan
// section 7.3 last two paragraphs). A page whose event stream drops keeps its render-time
// baseline; when the stream comes back, the first `state` event is compared with it and the
// page reloads once if the revision or the server instance changed, and not at all if nothing
// did. A restart therefore costs one recovery reload, never a loop. A network error is retried
// by the browser itself; a refused stream (a 503 at the stream limit) closes the EventSource,
// and only the client's own re-creation with backoff brings it back, tested separately.
//
// Waits are `expect.poll` and locator assertions; nothing here sleeps. "Exactly one reload"
// is a navigation count that reaches its value and then holds over an observation window,
// the polled-window pattern of reload.spec.ts.
import type { Page } from '@playwright/test';
import { copyFixtureSite, expect, startPreview, test, trackNavigations } from './support/preview';

const EVENTS = '**/__site_preview/events';
const EVENTS_PATH = '/__site_preview/events';
const CLIENT = 'script[src*="__site_preview/client.js"]';

/** Fails as soon as the navigation count leaves `expected`; passes once `ms` have held. */
async function expectStable(navigations: { count: () => number }, expected: number, ms = 2_000) {
  const started = Date.now();
  await expect
    .poll(
      () => {
        expect(navigations.count(), 'a reload loop or a missed reload').toBe(expected);
        return Date.now() - started >= ms;
      },
      { intervals: [100], timeout: ms + 10_000 },
    )
    .toBe(true);
}

/** Counts event-stream responses with status 200, the moments a stream (re)connected. */
function trackStreams(page: Page): { opened: () => number } {
  let opened = 0;
  page.on('response', (response) => {
    if (new URL(response.url()).pathname === EVENTS_PATH && response.status() === 200) opened += 1;
  });
  return { opened: () => opened };
}

/** The `data-revision` the server injects into the HTML it serves now, fetched past routes. */
async function servedRevision(page: Page, url: string): Promise<string | undefined> {
  const html = await (await page.request.get(url)).text();
  return /data-revision="([^"]*)"/.exec(html)?.[1];
}

/** The `index.html` of the fixture with its heading text replaced. */
async function retitled(site: { read: (path: string) => Promise<string> }, title: string) {
  return (await site.read('index.html')).replace('Route planner</h1>', `${title}</h1>`);
}

test('should_reload_once_when_stream_reconnects_after_an_edit_made_while_it_was_down', async ({
  page,
  preview,
  site,
}) => {
  // given: the stream never connects, so the page holds only its render-time baseline.
  const navigations = trackNavigations(page);
  const streams = trackStreams(page);
  let blocked = 0;
  await page.route(EVENTS, (route) => {
    blocked += 1;
    return route.abort();
  });
  await page.goto(preview.url);
  await expect(page.getByRole('heading', { name: 'Route planner' })).toBeVisible();
  await expect.poll(() => blocked, 'the client tried to connect').toBeGreaterThan(0);
  const rendered = await page.locator(CLIENT).getAttribute('data-revision');

  // when: the edit is published while disconnected, then the stream is let through.
  await site.write('index.html', await retitled(site, 'Trail planner'));
  await expect.poll(() => servedRevision(page, preview.url)).not.toBe(rendered);
  expect(navigations.count(), 'no reload without a stream').toBe(1);
  await page.unroute(EVENTS);

  // then
  await expect(page.getByRole('heading', { name: 'Trail planner' })).toBeVisible();
  await expect.poll(() => navigations.count()).toBe(2);
  // The reconnect that carried the recovery, then the reloaded page's own stream.
  await expect.poll(() => streams.opened(), 'the reloaded page connected').toBe(2);
  await expectStable(navigations, 2);
});

test('should_not_reload_when_stream_reconnects_to_an_unchanged_server', async ({
  page,
  preview,
}) => {
  // given
  const navigations = trackNavigations(page);
  const streams = trackStreams(page);
  let blocked = 0;
  await page.route(EVENTS, (route) => {
    blocked += 1;
    return route.abort();
  });
  await page.goto(preview.url);
  await expect(page.getByRole('heading', { name: 'Route planner' })).toBeVisible();
  await expect.poll(() => blocked, 'the client tried to connect').toBeGreaterThan(0);

  // when
  await page.unroute(EVENTS);
  await expect.poll(() => streams.opened(), 'the client reconnected').toBe(1);

  // then: the first state matches the baseline, so the page stays.
  await expectStable(navigations, 1);
  await expect(page.getByRole('heading', { name: 'Route planner' })).toBeVisible();
});

test('should_retry_and_reload_once_when_stream_was_refused_with_503_during_an_edit', async ({
  page,
  preview,
  site,
}) => {
  // given: every stream request is answered 503, as at the stream limit (plan 7.4). A non-200
  // answer closes an EventSource for good, so only the client's own re-creation retries it.
  const navigations = trackNavigations(page);
  const streams = trackStreams(page);
  let refused = 0;
  await page.route(EVENTS, (route) => {
    refused += 1;
    return route.fulfill({ status: 503, body: 'too many streams' });
  });
  await page.goto(preview.url);
  await expect(page.getByRole('heading', { name: 'Route planner' })).toBeVisible();
  await expect.poll(() => refused, 'the client retried after a 503').toBeGreaterThanOrEqual(2);
  const rendered = await page.locator(CLIENT).getAttribute('data-revision');

  // when
  await site.write('index.html', await retitled(site, 'Trail planner'));
  await expect.poll(() => servedRevision(page, preview.url)).not.toBe(rendered);
  expect(navigations.count(), 'no reload without a stream').toBe(1);
  await page.unroute(EVENTS);

  // then
  await expect(page.getByRole('heading', { name: 'Trail planner' })).toBeVisible();
  await expect.poll(() => navigations.count()).toBe(2);
  await expect.poll(() => streams.opened(), 'the reloaded page connected').toBe(2);
  await expectStable(navigations, 2);
});

test('should_reload_once_and_stay_when_server_restarts_on_the_same_port', async ({
  page,
  preview,
}) => {
  // given: a connected page.
  const navigations = trackNavigations(page);
  const streams = trackStreams(page);
  const port = preview.port;
  await page.goto(preview.url);
  await expect.poll(() => streams.opened(), 'the stream connected').toBe(1);
  const instance = await page.locator(CLIENT).getAttribute('data-instance');
  expect(instance, 'the injected instance id').toBeTruthy();

  // when: a clean stop and a new process on the same explicit port, unchanged files.
  await preview.restart();
  expect(preview.port).toBe(port);

  // then: one recovery reload onto the new instance, and no loop after it.
  await expect.poll(() => navigations.count()).toBe(2);
  await expect(page.locator(CLIENT)).not.toHaveAttribute('data-instance', String(instance));
  // The first stream, the reconnect to the new process, then the reloaded page's own stream.
  await expect.poll(() => streams.opened(), 'the reloaded page connected').toBe(3);
  await expectStable(navigations, 2, 3_000);
  await expect(page.getByRole('heading', { name: 'Route planner' })).toBeVisible();
});

test('should_show_edit_made_while_server_was_stopped_when_it_restarts_on_the_same_port', async ({
  page,
}) => {
  // given: a connected page on a server this test stops itself.
  const site = await copyFixtureSite();
  let preview = await startPreview(site.dir);
  try {
    const navigations = trackNavigations(page);
    const streams = trackStreams(page);
    const port = preview.port;
    await page.goto(preview.url);
    await expect.poll(() => streams.opened(), 'the stream connected').toBe(1);
    await preview.stop();

    // when: the edit lands while nothing serves or watches the site.
    await site.write('index.html', await retitled(site, 'Offline edit'));
    preview = await startPreview(site.dir, { port });

    // then
    await expect(page.getByRole('heading', { name: 'Offline edit' })).toBeVisible();
    await expect.poll(() => navigations.count()).toBe(2);
    await expectStable(navigations, 2, 3_000);
    await preview.stop();
  } finally {
    await preview.kill();
    await site.dispose();
  }
});

test('should_reload_once_when_server_is_killed_and_restarted_on_the_same_port', async ({
  page,
}) => {
  // given: a connected page on a server this test kills itself.
  const site = await copyFixtureSite();
  let preview = await startPreview(site.dir);
  try {
    const navigations = trackNavigations(page);
    const streams = trackStreams(page);
    const port = preview.port;
    await page.goto(preview.url);
    await expect.poll(() => streams.opened(), 'the stream connected').toBe(1);
    const instance = await page.locator(CLIENT).getAttribute('data-instance');
    expect(instance, 'the injected instance id').toBeTruthy();

    // when: no clean shutdown, so the stream ends without the server closing it.
    await preview.kill();
    preview = await startPreview(site.dir, { port });

    // then
    await expect.poll(() => navigations.count()).toBe(2);
    await expect(page.locator(CLIENT)).not.toHaveAttribute('data-instance', String(instance));
    await expectStable(navigations, 2, 3_000);
    await preview.stop();
  } finally {
    await preview.kill();
    await site.dispose();
  }
});
