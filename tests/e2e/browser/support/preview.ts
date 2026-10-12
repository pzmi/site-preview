// The browser-test harness (E2E-04, plan section 12 "Browser integration test").
//
// It runs the real `site-preview` binary, never a stand-in, against a tempdir copy of the
// shared fixture site (`tests/fixtures/site`, ADR 0016). The server starts with
// `--json --port 0`; the harness reads the first stdout line, the `ready` event, and takes the
// URL from it exactly as an agent would. Teardown stops the process with SIGTERM and asserts a
// clean exit, so a hung or crashing shutdown fails the test that caused it.
//
// Specs import `test` and `expect` from here instead of from `@playwright/test`:
//
//   test('...', async ({ page, preview, site }) => {
//     await page.goto(preview.url);
//     await site.write('index.html', '<h1>changed</h1>');
//     await expect(page.locator('h1')).toHaveText('changed');
//   });
//
// Fixtures: `site` (a tempdir copy of the fixture site) and `preview` (a running server on
// it). Helpers: `startPreview`, `copyFixtureSite`, `trackNavigations`. Waits are event-driven
// (`expect` polling, process events); the only timers are deadlines that bound a failure.
import { test as base, expect, type Page } from '@playwright/test';
import { type ChildProcess, spawn } from 'node:child_process';
import { once } from 'node:events';
import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

export { expect };

/** The repository root: this file is `tests/e2e/browser/support/preview.ts`. */
export const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../../../..');

/** The committed fixture site, copied (never served in place) by every test. */
export const FIXTURE_SITE = join(REPO_ROOT, 'tests/fixtures/site');

/** How long the harness waits for `ready`, and for a SIGTERM exit, before failing. */
const DEADLINE_MS = 30_000;

/** One JSON line the server printed on stdout. Unknown fields are kept and ignored. */
export interface PreviewEvent {
  event: string;
  [field: string]: unknown;
}

/** The `ready` event, `{"event":"ready","url":...,"pid":...,"watching":...}`. */
export interface ReadyEvent extends PreviewEvent {
  event: 'ready';
  url: string;
  pid: number;
  watching: boolean;
}

/**
 * The binary under test: `$SITE_PREVIEW_BIN` if set, else `site-preview` in the debug profile
 * of `$CARGO_TARGET_DIR` (relative paths are against the repository root) or of `target/`.
 */
export function binaryPath(): string {
  const override = process.env.SITE_PREVIEW_BIN;
  if (override) return override;
  const target = process.env.CARGO_TARGET_DIR ?? 'target';
  const dir = isAbsolute(target) ? target : join(REPO_ROOT, target);
  return join(dir, 'debug', 'site-preview');
}

/** A tempdir copy of the fixture site with small file helpers. Paths are site-relative. */
export class Site {
  constructor(readonly dir: string) {}

  /** Writes (creating parent directories) a file in the site. */
  async write(relative: string, content: string | Buffer): Promise<void> {
    const path = join(this.dir, relative);
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, content);
  }

  /** Reads a file of the site as UTF-8. */
  async read(relative: string): Promise<string> {
    return readFile(join(this.dir, relative), 'utf8');
  }

  /** Deletes a file or directory of the site. */
  async remove(relative: string): Promise<void> {
    await rm(join(this.dir, relative), { recursive: true, force: true });
  }

  /** Deletes the whole tempdir. */
  async dispose(): Promise<void> {
    await rm(this.dir, { recursive: true, force: true });
  }
}

/** Copies `tests/fixtures/site` into a fresh tempdir. */
export async function copyFixtureSite(): Promise<Site> {
  const dir = await mkdtemp(join(tmpdir(), 'site-preview-browser-'));
  await cp(FIXTURE_SITE, dir, { recursive: true });
  return new Site(dir);
}

export interface StartOptions {
  /** The port to listen on; 0 (the default) lets the system choose. */
  port?: number;
  /** Extra `serve` flags, for example `['--debounce', '50ms']`. */
  args?: string[];
}

/** A running `site-preview serve --json` process. */
export class PreviewServer {
  /** The URL from the `ready` event, with a trailing slash: `http://127.0.0.1:PORT/`. */
  url!: string;
  /** The port in `url`; pass it as `port` to restart on the same one. */
  port!: number;
  /** The `ready` event of the current process. */
  ready!: ReadyEvent;
  /** Every JSON line printed so far by the current process, in order. */
  events: PreviewEvent[] = [];

  private child!: ChildProcess;
  private stderr = '';
  private exit?: Promise<{ code: number | null; signal: NodeJS.Signals | null }>;
  private listeners = new Set<() => void>();

  constructor(
    readonly siteDir: string,
    private readonly options: StartOptions = {},
  ) {}

  /** Spawns the process and waits for `ready`. Called by [`startPreview`]. */
  async launch(port: number): Promise<void> {
    const bin = binaryPath();
    const args = ['serve', this.siteDir, '--json', '--port', String(port), ...(this.options.args ?? [])];
    this.events = [];
    this.stderr = '';
    const child = spawn(bin, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    this.child = child;
    // 'exit' fires once, also when the binary is missing ('error' then), so wait on both.
    this.exit = new Promise((done) => {
      child.once('error', () => done({ code: null, signal: null }));
      child.once('exit', (code, signal) => done({ code, signal }));
    });
    child.stderr!.setEncoding('utf8');
    child.stderr!.on('data', (chunk: string) => (this.stderr += chunk));
    const lines = createInterface({ input: child.stdout! });
    lines.on('line', (line) => {
      try {
        const parsed: unknown = JSON.parse(line);
        if (parsed && typeof parsed === 'object' && typeof (parsed as PreviewEvent).event === 'string') {
          this.events.push(parsed as PreviewEvent);
          for (const wake of this.listeners) wake();
        }
      } catch {
        // Not a JSON event line; stderr and the exit code carry the diagnosis.
      }
    });
    const ready = await this.waitForEvent((e) => e.event === 'ready', DEADLINE_MS).catch((error) => {
      const exitCode = child.exitCode;
      child.kill('SIGKILL');
      throw new PreviewStartError(
        `${bin} ${args.join(' ')}: no ready event: ${error}\nstderr: ${this.stderr}`,
        exitCode,
      );
    });
    this.ready = ready as ReadyEvent;
    this.url = this.ready.url;
    this.port = Number(new URL(this.url).port);
  }

  /** Resolves with the first event (already seen or future) the predicate accepts. */
  waitForEvent(match: (event: PreviewEvent) => boolean, timeoutMs = DEADLINE_MS): Promise<PreviewEvent> {
    return new Promise((resolveEvent, reject) => {
      const check = (): boolean => {
        const found = this.events.find(match);
        if (found) {
          cleanup();
          resolveEvent(found);
        }
        return Boolean(found);
      };
      const timer = setTimeout(() => {
        cleanup();
        reject(new Error(`no matching event within ${timeoutMs} ms; saw ${JSON.stringify(this.events)}`));
      }, timeoutMs);
      const onExit = () => {
        cleanup();
        reject(new Error(`the server exited first; stderr: ${this.stderr}`));
      };
      const cleanup = () => {
        clearTimeout(timer);
        this.listeners.delete(check);
        this.child.off('exit', onExit);
        this.child.off('error', onExit);
      };
      if (check()) return;
      this.listeners.add(check);
      this.child.once('exit', onExit);
      this.child.once('error', onExit);
    });
  }

  /** Whether the process is still running. */
  get running(): boolean {
    return this.child.exitCode === null && this.child.signalCode === null;
  }

  /** The server's stderr so far, for diagnostics. */
  get log(): string {
    return this.stderr;
  }

  /**
   * Stops the process with SIGTERM and asserts it exited with status 0. A process that had
   * already died, or that ignores SIGTERM, fails here.
   */
  async stop(): Promise<void> {
    if (!this.running) {
      const { code, signal } = await this.exit!;
      throw new Error(`the server had already exited (code ${code}, signal ${signal}); stderr: ${this.stderr}`);
    }
    this.child.kill('SIGTERM');
    const timer = setTimeout(() => this.child.kill('SIGKILL'), DEADLINE_MS);
    const { code, signal } = await this.exit!;
    clearTimeout(timer);
    expect(
      { code, signal },
      `the server must exit cleanly on SIGTERM; stderr: ${this.stderr}`,
    ).toEqual({ code: 0, signal: null });
  }

  /** Stops the server (cleanly) and starts a new process on the same site and the same port. */
  async restart(): Promise<void> {
    const port = this.port;
    await this.stop();
    await this.launch(port);
  }

  /** Kills the process without a clean-exit assertion; teardown after a failed test. */
  async kill(): Promise<void> {
    if (this.running) {
      this.child.kill('SIGKILL');
      await once(this.child, 'exit');
    }
  }
}

/** A start that produced no `ready` event; `exitCode` is the process's status if it had exited. */
export class PreviewStartError extends Error {
  constructor(
    message: string,
    readonly exitCode: number | null,
  ) {
    super(message);
    this.name = 'PreviewStartError';
  }
}

/** Starts the server on `siteDir` and waits for its `ready` event. */
export async function startPreview(siteDir: string, options: StartOptions = {}): Promise<PreviewServer> {
  const server = new PreviewServer(siteDir, options);
  await server.launch(options.port ?? 0);
  return server;
}

/**
 * Counts main-frame navigations of `page` (each `framenavigated` of the main frame, including
 * the first load). Use with `expect.poll` to assert "exactly N reloads" without a sleep.
 */
export function trackNavigations(page: Page): { count: () => number } {
  let count = 0;
  page.on('framenavigated', (frame) => {
    if (frame === page.mainFrame()) count += 1;
  });
  return { count: () => count };
}

export const test = base.extend<{ site: Site; preview: PreviewServer }>({
  site: async ({}, use) => {
    const site = await copyFixtureSite();
    await use(site);
    await site.dispose();
  },
  preview: async ({ site }, use, testInfo) => {
    const preview = await startPreview(site.dir);
    await use(preview);
    if (testInfo.status === 'passed' || testInfo.status === 'skipped') {
      await preview.stop();
    } else {
      // The test already failed: keep its error primary, but keep the server log as evidence.
      await testInfo.attach('server-stderr', { body: preview.log, contentType: 'text/plain' });
      await preview.kill();
    }
  },
});
