// Browser smoke for the served page (E2E-05, plan section 12 browser steps 3-5 and 8): the
// initial load, no reload loop, an edit reaching the page, clean consoles and requests, the
// asset fetches, screenshots at three widths, and a clean server shutdown. Reconnect and
// restart recovery are E2E-06; hot CSS/asset/JS updates are E2E-09.
//
// Waits are `expect.poll` and locator assertions; nothing here sleeps. The "no reload loop"
// window is the one place that observes for a duration, and it does so as a polled
// assertion that must keep holding, not as a timer.
import type { Page } from '@playwright/test';
import { copyFixtureSite, expect, startPreview, test, trackNavigations } from './support/preview';

/** Records page errors and failed or error-status requests to the origin. */
function watchProblems(page: Page, origin: string): { problems: () => string[] } {
  const problems: string[] = [];
  page.on('pageerror', (error) => problems.push(`pageerror: ${error.message}`));
  page.on('requestfailed', (request) =>
    problems.push(`requestfailed: ${request.url()} ${request.failure()?.errorText}`),
  );
  page.on('response', (response) => {
    if (response.url().startsWith(origin) && response.status() >= 400) {
      problems.push(`status ${response.status()}: ${response.url()}`);
    }
  });
  return { problems: () => problems };
}

test('should_show_initial_text_without_reload_loop_when_page_loads', async ({ page, preview }) => {
  // given
  const navigations = trackNavigations(page);

  // when: the window starts only once the page's event stream is open, so a reload loop
  // (which needs the stream) has had the chance to begin.
  const connected = page.waitForResponse((response) => response.url().endsWith('/__site_preview/events'));
  await page.goto(preview.url);
  await connected;

  // then
  await expect(page.getByRole('heading', { name: 'Route planner' })).toBeVisible();
  await expect(page.locator('#routes')).not.toBeEmpty();
  // Observe a 2 s window (measured by the page's own clock, not a sleep): every poll
  // fails the test if a second navigation appeared, and the poll ends once 2 s have passed.
  const started = await page.evaluate(() => performance.now());
  await expect
    .poll(
      async () => {
        expect(navigations.count(), 'a reload loop').toBe(1);
        return (await page.evaluate(() => performance.now())) - started >= 2_000;
      },
      { intervals: [100], timeout: 10_000 },
    )
    .toBe(true);
});

test('should_show_new_text_after_one_reload_when_html_is_edited', async ({ page, preview, site }) => {
  // given
  const navigations = trackNavigations(page);
  await page.goto(preview.url);
  await expect(page.getByRole('heading', { name: 'Route planner' })).toBeVisible();
  const html = await site.read('index.html');

  // when
  await site.write('index.html', html.replace('Route planner</h1>', 'Trail planner</h1>'));

  // then
  await expect(page.getByRole('heading', { name: 'Trail planner' })).toBeVisible();
  await expect.poll(() => navigations.count()).toBe(2);
});

test('should_report_no_errors_when_page_and_assets_load', async ({ page, preview }) => {
  // given
  const watched = watchProblems(page, new URL(preview.url).origin);
  const consoleErrors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text());
  });

  // when
  await page.goto(preview.url);
  await expect(page.locator('#routes')).not.toBeEmpty();
  await expect(page.getByRole('img', { name: 'Sample map tile' })).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => (document.querySelector('img') as HTMLImageElement).naturalWidth))
    .toBeGreaterThan(0);

  // then
  expect(watched.problems()).toEqual([]);
  expect(consoleErrors).toEqual([]);
});

test('should_serve_css_js_svg_and_gpx_when_fetched_from_the_page', async ({ page, preview }) => {
  // given
  await page.goto(preview.url);

  // when
  const results = await page.evaluate(async () => {
    const out: Record<string, { status: number; type: string | null; bytes: number }> = {};
    for (const path of ['styles.css', 'app.js', 'images/sample.svg', 'routes.gpx']) {
      const response = await fetch(path);
      out[path] = {
        status: response.status,
        type: response.headers.get('content-type'),
        bytes: (await response.arrayBuffer()).byteLength,
      };
    }
    return out;
  });

  // then
  for (const [path, result] of Object.entries(results)) {
    expect(result.status, path).toBe(200);
    expect(result.bytes, path).toBeGreaterThan(0);
  }
  expect(results['styles.css'].type).toContain('text/css');
  expect(results['app.js'].type).toMatch(/javascript/);
  expect(results['images/sample.svg'].type).toContain('image/svg+xml');
  // Stylesheet applied: the heading takes the fixture's accent colour.
  await expect(page.getByRole('heading', { name: 'Route planner' })).toHaveCSS(
    'color',
    'rgb(31, 111, 235)',
  );
});

for (const width of [390, 768, 1440]) {
  test(`should_render_without_horizontal_overflow_when_viewport_is_${width}px`, async ({
    page,
    preview,
  }, testInfo) => {
    // given
    await page.setViewportSize({ width, height: 800 });

    // when
    await page.goto(preview.url);
    await expect(page.locator('#routes')).not.toBeEmpty();
    const screenshot = await page.screenshot({ fullPage: true });
    await testInfo.attach(`screenshot-${width}`, { body: screenshot, contentType: 'image/png' });

    // then
    expect(screenshot.byteLength).toBeGreaterThan(0);
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
  });
}

test('should_exit_with_status_zero_when_sigterm_arrives_after_browsing', async ({ page }) => {
  // given
  const site = await copyFixtureSite();
  const preview = await startPreview(site.dir, { args: ['--debounce', '50ms'] });
  try {
    await page.goto(preview.url);
    await expect(page.locator('#routes')).not.toBeEmpty();

    // when / then: `stop` asserts exit code 0 and no signal within its deadline.
    await preview.stop();
    expect(preview.running).toBe(false);
  } finally {
    await preview.kill();
    await site.dispose();
  }
});
