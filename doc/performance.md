# Performance measurement

The targets of plan section 13 are measured by `cargo xtask bench` (E2E-08, D28, ADR 0029).
The command is not part of `check` and does not block a merge: a missed timing target is
reported and the run still exits 0. The targets become blocking only after the owner approves
a budget (foundation.md section 12, phase 5). A broken soak assertion (a leaked descriptor,
growing memory, a wrong connection cap) exits 1.

## Procedure

```bash
cargo xtask bench     # about 4 minutes, plus the release build on a cold target directory
```

The command:

1. builds `site-preview` with `cargo build --release --locked -p preview-cli`;
2. generates, in a fresh directory under the system temporary directory (removed on exit), a
   site of 1,000 served files (`index.html` and `sectionA/pageB/itemC.{html,css,js}`, three
   levels deep) beside an ignored `node_modules/pkgA/lib/fB.js` tree of 10,000 files;
3. runs each measurement below against a server started with `serve <site> --json` on a
   dynamic port, reading its URL from the `ready` event, and kills every server it spawned
   (by PID, also on failure);
4. writes `target/bench/report.json` (schema 1: `environment`, `metrics`, `checks`, raw
   `samples_ms`) and prints a verdict line per value.

| Measurement | Method |
|:--|:--|
| Start-up | Spawn to the `ready` line on stdout, median of 10 runs. |
| Reload latency | One SSE client; 3 uncounted warm-up saves, then 50 saves of a page. Time from just before the file write to the arrival of the `reload` event, median and p95 (nearest rank). The default 150 ms debounce is inside this figure. 100 ms between saves. |
| Idle CPU | A fresh server, 2 s to settle, then 30 s with no client: user plus system CPU time of the process, from `/proc/<pid>/stat` on Linux or `ps -o cputime=` on macOS, as a percentage of one core. |
| Soak | One save and client warm up the lazily opened descriptors, then the baseline descriptor count (`/proc/<pid>/fd`, `lsof` on macOS) and RSS (`VmRSS`, `ps -o rss=`) are taken. 10 SSE clients watch 200 saves; every client must receive every `reload`. RSS is sampled after save 20 (warm) and at the end. |
| Connection cap | After the soak the clients close and 64 streams are opened; the 65th must answer 503. All are closed, and the descriptor count must return to the baseline (polled for up to 5 s). |

Soak assertions: no leaked descriptors (count after at most the count before), stable RSS
(the end value at most the warm value plus a quarter, with a floor of 4 MiB of slack), every
save delivered to every client, and exactly 64 open streams before the 503.

Timing here is real time on a real filesystem, which is why this is a benchmark and not a
deterministic test; the numbers move with the machine.

## First recorded results

Run on 2026-10-13, commit of E2E-08, release build, `site-preview 0.1.0`, `rustc 1.97.0`.

Environment:

- CPU: Intel Xeon @ 2.80 GHz, 4 cores (x86_64).
- Kernel: Linux 6.18.44 (Firecracker microVM guest).
- Filesystem of the generated site: the system temporary directory, an ext4 root
  (`stat -f` prints the ext family as `ext2/ext3`).
- **This is a shared, loaded sandbox**: the 1, 5 and 15 minute load averages were
  25.4 / 19.2 / 15.8 at the start and 24.5 / 21.9 / 17.2 at the end, on 4 cores, because other
  builds and test runs shared the machine. The figures below are therefore pessimistic for
  an idle development laptop, not optimistic; they are not a quiet-machine baseline.

| Value | Measured | Plan section 13 target | Verdict |
|:--|:--|:--|:--|
| Start-up, median of 10 (1,000 files, ignored 10k `node_modules`) | 14.3 ms (max 29.9 ms) | under 1 s | met |
| Save to `reload` event, median of 50 | 155.9 ms | about 500 ms | met |
| Save to `reload` event, p95 of 50 | 180.0 ms (max 180.6 ms) | about 500 ms | met |
| Idle CPU over 30 s | 0.00 % of one core (below the 10 ms tick resolution) | close to zero | met (threshold used here: 1 %) |
| Descriptors, before / with 10 clients / after soak | 14 / 24 / 14 | no leak | met |
| RSS, start / after save 20 / end of soak | 5,752 / 5,920 / 6,808 KiB | stable | met (+15 % over warm; the limit is the larger of +25 % and 4 MiB, which detects a leak of about 22 KiB per save or more) |
| Soak delivery | 200 of 200 saves reached all 10 clients | n/a | met |
| Connection cap | 64 streams open, the 65th answered 503 | maximum 64 | met |

No target was missed. Notes for owner review:

- The 1 % idle threshold is this procedure's reading of "close to zero", not a number in the
  plan; the owner should confirm it or set a budget.
- The latency is dominated by the 150 ms default debounce (median 155.9 ms), so the
  measured watcher and delivery overhead is about 6 ms; a larger `--debounce` adds to it
  one for one.
- Start-up is measured to the `ready` line, after the listener accepts connections, as plan
  section 13 words it ("measured after binary launch"); the ignored `node_modules` is never
  walked for serving, and the watcher registration cost for it is inside the figure.
- The loaded machine makes the raw samples (`samples_ms` in the report) noisy; repeat on a
  quiet machine before using these as a budget baseline.
- Not measured here: the 8 MiB HTML rewrite bound, streamed binary reads and the 128
  connection limit of plan section 13. They are behaviours with their own tests, not timings.
- Only the native watcher on a local filesystem was measured; no OS watch limit was hit.
