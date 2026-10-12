// Browser proof of hot reload (E2E-09, ADR 0030 and 0031, plan sections 7 and 12): a stylesheet
// change is swapped in place, an image change is cache-busted in place, a script change reloads
// once and restores the scroll position from `history.state.__sitePreview`, a page can accept a
// script change through `window.__sitePreview.accept`, and anything else reloads once.
//
// Every test puts a random `window.__marker` on the page: it survives only if the document was
// never replaced, so "marker still there and one navigation" is the proof of "no reload". Edits
// start only after the page's event stream is open (the server subscribes before it answers),
// so the first edit is never mistaken for a missed one. Nothing sleeps: waits are polled
// assertions, and "nothing else happens" is a polled window measured by the page's own clock.
import { randomUUID } from 'node:crypto';
import type { Page } from '@playwright/test';
import {
  expect,
  startPreview,
  test as base,
  trackNavigations,
  type PreviewServer,
  type Site,
} from './support/preview';

/**
 * `base` plus `seeded`: writes the given files into the site and only then starts the server.
 * Files written after the start are changes the page may or may not have been served before,
 * which would make the first edit of a test racy.
 */
const test = base.extend<{ seeded: (files: Record<string, string>) => Promise<PreviewServer> }>({
  seeded: async ({ site }, use, testInfo) => {
    let started: PreviewServer | undefined;
    await use(async (files) => {
      for (const [path, content] of Object.entries(files)) await site.write(path, content);
      started = await startPreview(site.dir);
      return started;
    });
    if (!started) return;
    if (testInfo.status === 'passed') await started.stop();
    else await started.kill();
  },
});

const EVENTS = '/__site_preview/events';

/** Opens `path` with the event stream connected and a fresh marker on the window. */
async function open(page: Page, url: string, path = '') {
  const navigations = trackNavigations(page);
  // `framenavigated` also fires for same-document history changes, which a script reload makes
  // itself (it stores the scroll position with `replaceState`), so reload tests count loads.
  let documents = 0;
  page.on('load', () => (documents += 1));
  const loads = { count: () => documents };
  const connected = page.waitForResponse((response) => response.url().endsWith(EVENTS));
  await page.goto(new URL(path, url).href);
  await connected;
  const marker = randomUUID();
  await page.evaluate((value) => {
    (window as unknown as { __marker: string }).__marker = value;
  }, marker);
  return { navigations, loads, marker };
}

const markerOf = (page: Page) =>
  page.evaluate(() => (window as unknown as { __marker?: string }).__marker ?? null);

/**
 * Fails if the page navigates during `ms` of the page's clock. `expect.poll` alone would return
 * on its first success, so each poll asserts and the poll ends only when the window is over.
 */
async function expectQuietFor(page: Page, navigations: { count: () => number }, expected: number, ms = 1_000) {
  const started = await page.evaluate(() => performance.now());
  await expect
    .poll(
      async () => {
        expect(navigations.count(), 'an unexpected navigation').toBe(expected);
        return (await page.evaluate(() => performance.now())) - started >= ms;
      },
      { intervals: [100], timeout: 10_000 },
    )
    .toBe(true);
}

const page_ = (body: string, head = '') =>
  `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Hot</title>${head}</head>` +
  `<body>${body}</body></html>`;

async function writeStyledPage(site: Site, color: string) {
  await site.write('styles.css', `h1 { color: ${color}; }\n`);
}

test('should_swap_stylesheet_without_navigation_when_css_changes', async ({ page, preview, site }) => {
  // given
  const { navigations, marker } = await open(page, preview.url);
  const heading = page.getByRole('heading', { name: 'Route planner' });
  await expect(heading).toHaveCSS('color', 'rgb(31, 111, 235)');
  // Records the number of stylesheet links after every change of the head: the old link must
  // stay until the new one has loaded, so the count never reaches 0 (no unstyled flash).
  await page.evaluate(() => {
    const seen: number[] = [];
    (window as unknown as { __linkCounts: number[] }).__linkCounts = seen;
    new MutationObserver(() =>
      seen.push(document.querySelectorAll('link[rel~="stylesheet"]').length),
    ).observe(document.documentElement, { childList: true, subtree: true });
  });

  // when
  await writeStyledPage(site, 'rgb(200, 30, 60)');

  // then
  await expect(heading).toHaveCSS('color', 'rgb(200, 30, 60)');
  await expect(page.locator('link[rel~="stylesheet"]')).toHaveAttribute('href', /styles\.css\?__sp=\d+/);
  expect(await page.locator('link[rel~="stylesheet"]').count()).toBe(1);
  const counts = await page.evaluate(() => (window as unknown as { __linkCounts: number[] }).__linkCounts);
  expect(counts.length, 'the swap changed the head').toBeGreaterThan(0);
  expect(Math.min(...counts), 'a moment without a stylesheet').toBeGreaterThanOrEqual(1);
  expect(await markerOf(page)).toBe(marker);
  await expectQuietFor(page, navigations, 1);
});

test('should_swap_importing_link_without_navigation_when_imported_css_changes', async ({
  page,
  seeded,
  site,
}) => {
  // given
  const preview = await seeded({
    'theme.css': 'h1 { color: rgb(1, 2, 3); }\n',
    'main.css': '@import "theme.css";\nbody { margin: 0; }\n',
    'imports.html': page_('<h1>Imports</h1>', '<link rel="stylesheet" href="main.css">'),
  });
  const { navigations, marker } = await open(page, preview.url, 'imports.html');
  const heading = page.getByRole('heading', { name: 'Imports' });
  await expect(heading).toHaveCSS('color', 'rgb(1, 2, 3)');

  // when
  await site.write('theme.css', 'h1 { color: rgb(4, 5, 6); }\n');

  // then
  await expect(heading).toHaveCSS('color', 'rgb(4, 5, 6)');
  await expect(page.locator('link[rel~="stylesheet"]')).toHaveAttribute('href', /main\.css\?__sp=\d+/);
  expect(await markerOf(page)).toBe(marker);
  await expectQuietFor(page, navigations, 1);
});

test('should_cache_bust_image_without_navigation_when_asset_changes', async ({ page, preview, site }) => {
  // given
  const { navigations, marker } = await open(page, preview.url);
  const image = page.getByRole('img', { name: 'Sample map tile' });
  await expect.poll(() => image.evaluate((img: HTMLImageElement) => img.naturalWidth)).toBe(64);

  // when
  await site.write(
    'images/sample.svg',
    '<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128"><rect width="128" height="128" fill="#fed"/></svg>',
  );

  // then
  await expect.poll(() => image.evaluate((img: HTMLImageElement) => img.naturalWidth)).toBe(128);
  expect(await image.evaluate((img: HTMLImageElement) => img.currentSrc)).toMatch(/sample\.svg\?__sp=\d+/);
  await expect(page.locator('link[rel~="icon"]')).toHaveAttribute('href', /sample\.svg\?__sp=\d+/);
  expect(await markerOf(page)).toBe(marker);
  await expectQuietFor(page, navigations, 1);
});

test('should_reload_once_when_unreferenced_asset_changes', async ({ page, preview, site }) => {
  // given
  const { loads } = await open(page, preview.url);

  // when
  await site.write('routes.gpx', '<gpx version="1.1"><trk><name>Changed</name></trk></gpx>');

  // then
  await expect.poll(() => loads.count()).toBe(2);
  expect(await markerOf(page)).toBeNull();
  await expectQuietFor(page, loads, 2);
});

test('should_reload_once_and_restore_scroll_when_script_changes', async ({ page, seeded, site }) => {
  // given
  const preview = await seeded({
    'long.html': page_(
      '<h1>Long page</h1><div style="height:4000px"></div><p id="end">End</p><script src="app.js"></script>',
    ),
  });
  const { loads } = await open(page, preview.url, 'long.html');
  await page.evaluate(() => {
    window.scrollTo(0, 1500);
    // The document is replaced by the reload, but the window of a tab is kept: hand the state
    // the client stored just before it unloaded over to the next document.
    window.addEventListener('pagehide', () => {
      window.name = JSON.stringify(history.state);
    });
  });
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(1500);

  // when
  await site.write('app.js', '"use strict";\n// changed\n');

  // then
  await expect.poll(() => loads.count()).toBe(2);
  await expect.poll(() => page.evaluate(() => Math.abs(window.scrollY - 1500))).toBeLessThanOrEqual(2);
  const stored = JSON.parse(await page.evaluate(() => window.name));
  expect(stored.__sitePreview.scrollY).toBe(1500);
  expect(stored.__sitePreview.scrollX).toBe(0);
  // The marker is removed once the position is restored, and nothing else was touched.
  await expect.poll(() => page.evaluate(() => history.state)).toBeNull();
  expect(page.url()).toBe(new URL('long.html', preview.url).href);
  await expectQuietFor(page, loads, 2);
});

const WIDGET_V1 = 'export const render = (el) => { el.textContent = "widget v1"; };\n';
const ACCEPT_PAGE = page_(
  '<p id="out">starting</p>' +
    '<script type="module">' +
    'const out = document.getElementById("out");' +
    'const run = async (url) => (await import(url)).render(out);' +
    // Registered before the first await: a module with top-level await may finish after
    // DOMContentLoaded, and the client exists only from then on (client.js, top comment).
    'window.addEventListener("DOMContentLoaded", () => {' +
    '  window.__sitePreview.accept("/widget.js", ({ url }) => run(url));' +
    '  window.__sitePreview.accept("/broken.js", () => { throw new Error("cannot handle"); });' +
    '});' +
    'await run("/widget.js");' +
    '</script>',
);

test('should_run_handler_without_navigation_when_page_accepts_script_change', async ({
  page,
  seeded,
  site,
}) => {
  // given
  const preview = await seeded({ 'widget.js': WIDGET_V1, 'accept.html': ACCEPT_PAGE });
  const { navigations, marker } = await open(page, preview.url, 'accept.html');
  await expect(page.locator('#out')).toHaveText('widget v1');

  // when
  await site.write('widget.js', 'export const render = (el) => { el.textContent = "widget v2"; };\n');

  // then
  await expect(page.locator('#out')).toHaveText('widget v2');
  expect(await markerOf(page)).toBe(marker);
  await expectQuietFor(page, navigations, 1);
});

test('should_reload_once_when_accept_handler_throws', async ({ page, seeded, site }) => {
  // given
  const preview = await seeded({
    'widget.js': WIDGET_V1,
    'broken.js': '// v1\n',
    'accept.html': ACCEPT_PAGE,
  });
  const { loads } = await open(page, preview.url, 'accept.html');
  await expect(page.locator('#out')).toHaveText('widget v1');
  await expect.poll(() => page.evaluate(() => typeof (window as unknown as { __sitePreview?: { accept?: unknown } }).__sitePreview?.accept)).toBe('function');

  // when
  await site.write('broken.js', '// v2\n');

  // then
  await expect.poll(() => loads.count()).toBe(2);
  expect(await markerOf(page)).toBeNull();
  await expectQuietFor(page, loads, 2);
});

test('should_reload_once_when_script_change_is_not_accepted', async ({ page, seeded, site }) => {
  // given
  const preview = await seeded({
    'widget.js': WIDGET_V1,
    'other.js': '// v1\n',
    'accept.html': ACCEPT_PAGE,
  });
  const { loads } = await open(page, preview.url, 'accept.html');
  await expect(page.locator('#out')).toHaveText('widget v1');

  // when
  await site.write('other.js', '// v2\n');

  // then
  await expect.poll(() => loads.count()).toBe(2);
  await expectQuietFor(page, loads, 2);
});

test('should_reload_once_when_html_and_css_change_together', async ({ page, preview, site }) => {
  // given
  const { loads } = await open(page, preview.url);
  const html = await site.read('index.html');

  // when
  await site.write('index.html', html.replace('Route planner</h1>', 'Mixed change</h1>'));
  await writeStyledPage(site, 'rgb(9, 8, 7)');

  // then
  const heading = page.getByRole('heading', { name: 'Mixed change' });
  await expect(heading).toBeVisible();
  await expect(heading).toHaveCSS('color', 'rgb(9, 8, 7)');
  expect(await markerOf(page)).toBeNull();
  await expectQuietFor(page, loads, 2);
});

test('should_reload_once_when_css_and_script_change_in_quick_succession', async ({
  page,
  preview,
  site,
}) => {
  // given
  const { loads } = await open(page, preview.url);

  // when
  await writeStyledPage(site, 'rgb(7, 7, 7)');
  await site.write('app.js', '"use strict";\n// quick\n');

  // then
  await expect.poll(() => loads.count()).toBe(2);
  await expect(page.getByRole('heading', { name: 'Route planner' })).toHaveCSS('color', 'rgb(7, 7, 7)');
  expect(await markerOf(page)).toBeNull();
  await expectQuietFor(page, loads, 2);
});
