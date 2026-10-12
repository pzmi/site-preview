// The agent workflow (E2E-07, plan section 10, M4 exit gate): an automated agent starts the
// server, discovers its URL, inspects the page, edits, confirms the reload and shuts the
// server down, using only what AGENTS.md documents. This spec plays the agent against the real
// binary and also reads AGENTS.md, so the document cannot drift from behaviour: a command added
// to or removed from AGENTS.md without a matching row below fails the first test, and the
// documented JSON example is compared with what the server really prints.
//
// Every command in AGENTS.md is exercised here (the table is checked against the document):
//
//   | Command in AGENTS.md                           | Exercised by                                   |
//   |:--|:--|
//   | site-preview serve --json --port 0 <root>      | startPreview (spawns it with these flags)      |
//   | curl -fsS <url>                                | GET of `url` via fetch: status 200 (step 4)    |
//   | curl -sN <url>__site_preview/events            | EventTap: streaming GET of the events path     |
//   | kill -TERM <pid>                               | PreviewServer.stop: SIGTERM, exit status 0     |
//
// `curl` is not required to be installed; the fetch calls are the same requests. Waits are
// `expect.poll` and locator assertions; nothing here sleeps.
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { copyFixtureSite, expect, PreviewStartError, REPO_ROOT, startPreview, test } from './support/preview';

const AGENTS_MD = join(REPO_ROOT, 'AGENTS.md');
const SKILL_MD = join(REPO_ROOT, 'examples/claude-skill/static-html-preview/SKILL.md');
const CLIENT = 'script[src*="__site_preview/client.js"]';
const EVENTS_PATH = '__site_preview/events';

/** The rows of the table above: documented command (placeholders as written) -> exercised by. */
const EXERCISED = new Map<string, string>([
  ['site-preview serve --json --port 0 <root>', 'startPreview'],
  ['curl -fsS <url>', 'fetch of the url'],
  ['curl -sN <url>__site_preview/events', 'EventTap'],
  ['kill -TERM <pid>', 'PreviewServer.stop'],
]);

/** The non-empty lines of every ```sh block of a Markdown document, trimmed. */
function shellCommands(markdown: string): string[] {
  const commands: string[] = [];
  for (const block of markdown.matchAll(/```sh\n([\s\S]*?)```/g)) {
    for (const line of block[1].split('\n')) if (line.trim()) commands.push(line.trim());
  }
  return commands;
}

/** One server-sent event: its `event:` name and parsed `data:` JSON. */
interface SseEvent {
  event: string;
  data: { instance_id: string; revision: number };
}

/** A streaming GET of the events path, the way `curl -sN <url>__site_preview/events` reads it. */
class EventTap {
  readonly events: SseEvent[] = [];
  private readonly abort = new AbortController();
  private done: Promise<void> = Promise.resolve();

  async open(url: string): Promise<void> {
    const response = await fetch(`${url}${EVENTS_PATH}`, { signal: this.abort.signal });
    expect(response.status).toBe(200);
    expect(response.headers.get('content-type')).toContain('text/event-stream');
    this.done = this.read(response.body!.getReader());
  }

  private async read(reader: ReadableStreamDefaultReader<Uint8Array>): Promise<void> {
    const decoder = new TextDecoder();
    let buffer = '';
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) return;
        buffer += decoder.decode(value, { stream: true });
        let end: number;
        while ((end = buffer.indexOf('\n\n')) >= 0) {
          const frame = buffer.slice(0, end);
          buffer = buffer.slice(end + 2);
          const name = /^event: (.*)$/m.exec(frame)?.[1];
          const data = /^data: (.*)$/m.exec(frame)?.[1];
          if (name && data) this.events.push({ event: name, data: JSON.parse(data) });
        }
      }
    } catch {
      // Aborted by `close`.
    }
  }

  async close(): Promise<void> {
    this.abort.abort();
    await this.done;
  }
}

/** The `data-instance` and `data-revision` of the injected script in the HTML served now. */
async function served(url: string): Promise<{ instance?: string; revision?: number }> {
  const html = await (await fetch(url)).text();
  const revision = /data-revision="([^"]*)"/.exec(html)?.[1];
  return { instance: /data-instance="([^"]*)"/.exec(html)?.[1], revision: revision ? Number(revision) : undefined };
}

test('should_match_the_commands_in_agents_md_when_compared_with_what_this_spec_exercises', async () => {
  // given
  const agents = await readFile(AGENTS_MD, 'utf8');

  // when
  const documented = shellCommands(agents);

  // then
  expect([...documented].sort()).toEqual([...EXERCISED.keys()].sort());
});

test('should_print_the_documented_ready_line_when_started_as_agents_md_says', async ({ preview }) => {
  // given
  const agents = await readFile(AGENTS_MD, 'utf8');
  const example = JSON.parse(/^ {3}\{"event":"ready".*$/m.exec(agents)![0].trim());

  // when
  const ready = preview.ready;

  // then: the same fields in the same types as the documented line, and the documented
  // degraded/recovered watcher lines are the ones the README and runbook describe.
  expect(Object.keys(ready).sort()).toEqual(Object.keys(example).sort());
  for (const key of Object.keys(example)) expect(typeof ready[key], key).toBe(typeof example[key]);
  expect(ready.watching).toBe(true);
  expect(ready.url).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/$/);
  expect(() => process.kill(ready.pid, 0), 'pid is a live process').not.toThrow();
  expect(agents).toContain('{"event":"watcher","state":"degraded","reason":...}');
  expect(agents).toContain('{"event":"watcher","state":"watching"}');
  expect(agents).toContain('<url>__site_preview/events');
});

test('should_complete_the_whole_workflow_when_an_agent_follows_agents_md', async ({
  page,
  preview,
  site,
}, testInfo) => {
  // given: step 2-4, started by the fixture; the url answers and live reload is on.
  const { url, watching } = preview.ready;
  expect(watching).toBe(true);
  const reachable = await fetch(url);
  expect(reachable.status).toBe(200);
  const failures: string[] = [];
  const consoleErrors: string[] = [];
  page.on('console', (m) => m.type() === 'error' && consoleErrors.push(m.text()));
  page.on('pageerror', (e) => consoleErrors.push(e.message));
  // A navigation aborts the requests still in flight (the old page's event stream, a stylesheet
  // being revalidated); that is not a failed request. Refused connections and 4xx/5xx are.
  page.on(
    'requestfailed',
    (r) => r.failure()?.errorText !== 'net::ERR_ABORTED' && failures.push(`${r.url()} ${r.failure()?.errorText}`),
  );
  page.on('response', (r) => r.status() >= 400 && failures.push(`${r.status()} ${r.url()}`));

  // when: step 5, inspect at three widths, then the full page, accessibility and controls.
  for (const width of [390, 768, 1440]) {
    await page.setViewportSize({ width, height: 800 });
    await page.goto(url);
    await expect(page.getByRole('heading', { name: 'Route planner' })).toBeVisible();
    const shot = await page.screenshot();
    await testInfo.attach(`screenshot-${width}`, { body: shot, contentType: 'image/png' });
    expect(shot.byteLength).toBeGreaterThan(0);
  }
  const full = await page.screenshot({ fullPage: true });
  expect(full.byteLength).toBeGreaterThan(0);
  const snapshot = await page.locator('body').ariaSnapshot();
  expect(snapshot).toContain('heading "Route planner"');
  await page.getByRole('link', { name: 'Read the guide' }).click();
  await expect(page).toHaveURL(`${url}guide/`);
  await page.goBack();
  await expect(page.getByRole('heading', { name: 'Route planner' })).toBeVisible();

  // then
  expect(consoleErrors).toEqual([]);
  expect(failures).toEqual([]);

  // when: step 6, observe the reload through the event stream and through the page.
  const tap = new EventTap();
  await tap.open(url);
  await expect.poll(() => tap.events.map((e) => e.event)).toEqual(['state']);
  const baseline = tap.events[0].data;
  const rendered = Number(await page.locator(CLIENT).getAttribute('data-revision'));
  const instance = await page.locator(CLIENT).getAttribute('data-instance');
  expect(instance).toBe(baseline.instance_id);
  const html = await site.read('index.html');
  await site.write('index.html', html.replace('Route planner</h1>', 'Trail planner</h1>'));

  // then
  try {
    await expect.poll(() => tap.events.map((e) => e.event)).toContain('reload');
    const reload = tap.events.find((e) => e.event === 'reload')!;
    expect(reload.data.instance_id).toBe(baseline.instance_id);
    expect(reload.data.revision).toBeGreaterThan(baseline.revision);
    await expect(page.getByRole('heading', { name: 'Trail planner' })).toBeVisible();
    expect(Number(await page.locator(CLIENT).getAttribute('data-revision'))).toBeGreaterThan(rendered);
    await expect.poll(async () => (await served(url)).revision).toBe(reload.data.revision);
    expect((await served(url)).instance).toBe(instance);
  } finally {
    await tap.close();
  }
  // Step 8, SIGTERM and exit status 0, is asserted by the `preview` fixture's teardown.
});

test('should_stop_cleanly_when_a_check_fails_in_the_middle_of_the_workflow', async ({ page }) => {
  // given: the agent's pattern, start then try/finally stop, with a page that breaks a check.
  const site = await copyFixtureSite();
  await site.write(
    'index.html',
    (await site.read('index.html')).replace('</body>', '<script>throw new Error("broken");</script></body>'),
  );
  const preview = await startPreview(site.dir);
  let failure: unknown;
  try {
    const errors: string[] = [];
    page.on('pageerror', (e) => errors.push(e.message));
    await page.goto(preview.url);
    await expect.poll(() => errors).toContain('broken');

    // when: the check fails and throws inside the try.
    throw new Error('check failed: the page raised "broken"');
  } catch (error) {
    failure = error;
  } finally {
    // The original error stays primary: a throwing stop() must not mask it.
    await preview.stop().catch(() => preview.kill());
    await site.dispose();
  }

  // then: the check really failed, and the failure path still stopped the server.
  expect(String(failure)).toContain('check failed');
  expect(preview.running).toBe(false);
});

test('should_exit_with_status_1_and_no_ready_line_when_the_port_is_taken', async ({ preview, site }) => {
  // given: a server already on the port.
  const port = preview.port;

  // when: a second start on that exact port, as an agent would after a bad guess.
  const second = await startPreview(site.dir, { port }).catch((error: unknown) => error);

  // then: the agent gets exit status 1 (AGENTS.md) and a stderr message, not a hang.
  expect(second).toBeInstanceOf(PreviewStartError);
  expect((second as PreviewStartError).exitCode).toBe(1);
  expect((second as PreviewStartError).message).toMatch(/no ready event.*exited first/s);
  expect(preview.running).toBe(true);
});

test('should_ship_a_generic_skill_when_the_example_is_read', async () => {
  // given
  const skill = await readFile(SKILL_MD, 'utf8');

  // when
  const frontmatter = /^---\n([\s\S]*?)\n---\n/.exec(skill)?.[1] ?? '';

  // then: named after its directory, the same start command as AGENTS.md, nothing private.
  expect(frontmatter).toMatch(/^name: static-html-preview$/m);
  expect(frontmatter).toMatch(/^description:/m);
  expect(skill).toContain('site-preview serve --json --port 0 <root>');
  expect(skill).toContain('kill -TERM <pid>');
  expect(skill).not.toMatch(/\/home\/|\/Users\/|\/tmp\/|api[_-]?key|secret|token/i);
});
