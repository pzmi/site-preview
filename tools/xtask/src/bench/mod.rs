//! `cargo xtask bench`: the reproducible, non-blocking performance procedure of plan section
//! 13 (D28, ADR 0029, E2E-08). It builds the release binary, generates a site of 1,000 files
//! beside an ignored 10,000-file `node_modules`, and measures start-up, save-to-reload
//! latency, idle CPU and a soak, then writes `target/bench/report.json`.
//!
//! A missed timing target is reported and the command still exits 0: the targets become
//! blocking only after the owner approves a budget (D28). A broken soak assertion (a leaked
//! descriptor, growing memory, a wrong connection cap) or a run that cannot finish exits 1.
//! The results are recorded in `doc/performance.md`.

mod report;
mod server;
mod site;
mod stats;
mod sys;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

use report::{Check, Metric, Report, cpu_percent, rss_is_stable};
use server::{Opened, Server, SseClient};
use site::BenchSite;

/// Start-ups measured.
const STARTUPS: usize = 10;
/// Saves whose reload latency is measured, after [`LATENCY_WARMUP`] uncounted ones.
const LATENCY_SAVES: usize = 50;
const LATENCY_WARMUP: usize = 3;
/// Idle observation window.
const IDLE_SECONDS: u64 = 30;
/// Saves of the soak, and the event-stream clients watching them.
const SOAK_SAVES: usize = 200;
const SOAK_CLIENTS: usize = 10;
/// The save after which memory is considered warmed up.
const SOAK_WARM_AT: usize = 20;
/// The event-stream cap of plan section 13.
const STREAM_CAP: usize = 64;
/// Pause between saves, so one save's trailing events never count as the next.
const SAVE_GAP: Duration = Duration::from_millis(100);

/// Runs the benchmark and returns the exit code.
pub fn run() -> i32 {
    match measure() {
        Ok(report) => finish(&report),
        Err(err) => {
            eprintln!("xtask: bench: {err}");
            1
        }
    }
}

fn finish(report: &Report) -> i32 {
    let path = crate::repo::root().join(target_dir_name()).join("bench");
    let written = std::fs::create_dir_all(&path)
        .and_then(|()| std::fs::write(path.join("report.json"), report.to_json()));
    if let Err(err) = written {
        eprintln!("xtask: bench: cannot write the report: {err}");
        return 1;
    }
    for metric in &report.metrics {
        let verdict = if metric.met { "ok  " } else { "MISS" };
        eprintln!(
            "bench: {verdict} {:<22} {:>10.2} {:<3} (target {})",
            metric.name, metric.value, metric.unit, metric.target
        );
    }
    for check in &report.checks {
        let verdict = if check.passed { "ok  " } else { "FAIL" };
        eprintln!("bench: {verdict} {:<22} {}", check.name, check.detail);
    }
    eprintln!("bench: report at {}", path.join("report.json").display());
    if report.misses().is_empty() {
        eprintln!("bench: every timing target met");
    } else {
        eprintln!("bench: targets missed are for owner review (D28); they do not fail the run");
    }
    i32::from(!report.checks_passed())
}

/// `target`, or the directory `CARGO_TARGET_DIR` names (relative to the root).
fn target_dir_name() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| PathBuf::from("target"), PathBuf::from)
}

fn measure() -> Result<Report, String> {
    let binary = build_release()?;
    eprintln!("bench: generating the site");
    let site = BenchSite::generate()?;
    let mut report = Report {
        environment: environment(&binary, &site),
        ..Report::default()
    };
    startup(&binary, &site, &mut report)?;
    latency(&binary, &site, &mut report)?;
    idle(&binary, &site, &mut report)?;
    soak(&binary, &site, &mut report)?;
    report
        .environment
        .push(("load_average_end".to_owned(), load_average()));
    Ok(report)
}

fn build_release() -> Result<PathBuf, String> {
    eprintln!("bench: building the release binary");
    let status = Command::new("cargo")
        .args(["build", "--release", "--locked", "-p", "preview-cli"])
        .current_dir(crate::repo::root())
        .status()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if !status.success() {
        return Err("cargo build --release failed".to_owned());
    }
    let binary = crate::repo::root()
        .join(target_dir_name())
        .join("release/site-preview");
    binary
        .is_file()
        .then_some(binary.clone())
        .ok_or_else(|| format!("{} was not built", binary.display()))
}

fn startup(binary: &Path, site: &BenchSite, report: &mut Report) -> Result<(), String> {
    eprintln!("bench: start-up, {STARTUPS} runs");
    let mut samples = Vec::new();
    for _ in 0..STARTUPS {
        let server = Server::start(binary, site.root())?;
        samples.push(server.startup().as_secs_f64() * 1000.0);
    }
    let median = stats::median(&samples).ok_or("no start-up samples")?;
    report.metrics.push(Metric::at_most(
        "startup_median",
        median,
        "ms",
        1000.0,
        "< 1000 ms, spawn to ready (plan 13)",
    ));
    report.metrics.push(Metric::at_most(
        "startup_max",
        stats::max(&samples).unwrap_or(f64::NAN),
        "ms",
        1000.0,
        "< 1000 ms (informative)",
    ));
    report.samples.push(("startup".to_owned(), samples));
    Ok(())
}

/// Opens one client on `server` and consumes its baseline `state` event.
fn subscribe(server: &Server) -> Result<SseClient, String> {
    match SseClient::open(server.authority())? {
        Opened::Stream(mut client) => {
            client.wait_for("state")?;
            Ok(*client)
        }
        Opened::Refused(status) => Err(format!("the event stream answered {status}")),
    }
}

fn latency(binary: &Path, site: &BenchSite, report: &mut Report) -> Result<(), String> {
    eprintln!("bench: save-to-reload latency, {LATENCY_SAVES} saves");
    let server = Server::start(binary, site.root())?;
    let mut client = subscribe(&server)?;
    let mut samples = Vec::new();
    for revision in 0..LATENCY_WARMUP + LATENCY_SAVES {
        let begin = Instant::now();
        site.save(revision)?;
        client.wait_for("reload")?;
        if revision >= LATENCY_WARMUP {
            samples.push(begin.elapsed().as_secs_f64() * 1000.0);
        }
        sleep(SAVE_GAP);
    }
    let median = stats::median(&samples).ok_or("no latency samples")?;
    let p95 = stats::percentile(&samples, 95).ok_or("no latency samples")?;
    report.metrics.push(Metric::at_most(
        "reload_latency_median",
        median,
        "ms",
        500.0,
        "about 500 ms, save to reload event (plan 13), debounce 150 ms included",
    ));
    report.metrics.push(Metric::at_most(
        "reload_latency_p95",
        p95,
        "ms",
        500.0,
        "about 500 ms (plan 13)",
    ));
    report.metrics.push(Metric::at_most(
        "reload_latency_max",
        stats::max(&samples).unwrap_or(f64::NAN),
        "ms",
        500.0,
        "about 500 ms (informative)",
    ));
    report.samples.push(("reload_latency".to_owned(), samples));
    Ok(())
}

fn idle(binary: &Path, site: &BenchSite, report: &mut Report) -> Result<(), String> {
    eprintln!("bench: idle CPU over {IDLE_SECONDS} s");
    let server = Server::start(binary, site.root())?;
    // Let the initial scan and watcher registration finish before the window opens.
    sleep(Duration::from_secs(2));
    let before = sys::cpu_seconds(server.pid())?;
    let begin = Instant::now();
    sleep(Duration::from_secs(IDLE_SECONDS));
    let wall = begin.elapsed().as_secs_f64();
    let used = sys::cpu_seconds(server.pid())? - before;
    report.metrics.push(Metric::at_most(
        "idle_cpu",
        cpu_percent(used, wall),
        "%",
        1.0,
        "close to zero (plan 13); this run's threshold is 1% of one core",
    ));
    report.metrics.push(Metric::at_most(
        "idle_cpu_seconds",
        used,
        "s",
        f64::from(u32::try_from(IDLE_SECONDS).unwrap_or(30)) * 0.01,
        "CPU time in the window (informative)",
    ));
    Ok(())
}

fn soak(binary: &Path, site: &BenchSite, report: &mut Report) -> Result<(), String> {
    eprintln!("bench: soak, {SOAK_SAVES} saves, {SOAK_CLIENTS} clients");
    let server = Server::start(binary, site.root())?;
    let pid = server.pid();

    // Warm every lazily opened descriptor once, then take the baseline.
    let mut revision = 0;
    {
        let mut client = subscribe(&server)?;
        site.save(revision)?;
        client.wait_for("reload")?;
    }
    let fds_before = settled_fds(pid, 0)?;
    let rss_start = sys::rss_kib(pid)?;

    let mut clients = (0..SOAK_CLIENTS)
        .map(|_| subscribe(&server))
        .collect::<Result<Vec<_>, _>>()?;
    let mut rss_warm = rss_start;
    for save in 1..=SOAK_SAVES {
        revision += 1;
        site.save(revision)?;
        for client in &mut clients {
            client.wait_for("reload")?;
        }
        if save == SOAK_WARM_AT {
            rss_warm = sys::rss_kib(pid)?;
        }
        sleep(SAVE_GAP);
    }
    let fds_during = sys::fd_count(pid)?;
    drop(clients);

    let (open, refused) = cap_check(&server)?;
    let rss_end = sys::rss_kib(pid)?;
    let fds_after = settled_fds(pid, fds_before)?;

    report.checks.push(Check {
        name: "soak_delivery".to_owned(),
        passed: true,
        detail: format!("{SOAK_SAVES} saves reached all {SOAK_CLIENTS} clients"),
    });
    report.checks.push(Check {
        name: "soak_fds".to_owned(),
        passed: fds_after <= fds_before,
        detail: format!(
            "{fds_before} before, {fds_during} with {SOAK_CLIENTS} clients, {fds_after} after"
        ),
    });
    report.checks.push(Check {
        name: "soak_rss".to_owned(),
        passed: rss_is_stable(rss_warm, rss_end),
        detail: format!(
            "{rss_start} KiB at start, {rss_warm} KiB after save {SOAK_WARM_AT}, {rss_end} KiB at the end"
        ),
    });
    report.checks.push(Check {
        name: "stream_cap".to_owned(),
        passed: open == STREAM_CAP && refused == Some(503),
        detail: format!("{open} streams open; the next answered {refused:?}"),
    });
    report.metrics.push(Metric::at_most(
        "soak_rss_end",
        rss_end as f64 / 1024.0,
        "MiB",
        f64::MAX,
        "informative",
    ));
    Ok(())
}

/// Opens streams until the server refuses one; returns how many were open and the status of
/// the refusal. Everything is closed on return.
fn cap_check(server: &Server) -> Result<(usize, Option<u16>), String> {
    let mut open = Vec::new();
    for _ in 0..=STREAM_CAP {
        match SseClient::open(server.authority())? {
            Opened::Stream(mut client) => {
                client.wait_for("state")?;
                open.push(client);
            }
            Opened::Refused(status) => return Ok((open.len(), Some(status))),
        }
    }
    Ok((open.len(), None))
}

/// The descriptor count once the server has closed what clients dropped: polled for up to
/// 5 s until it is at most `target` (0: take the first reading that stops changing).
fn settled_fds(pid: u32, target: usize) -> Result<usize, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last = sys::fd_count(pid)?;
    while Instant::now() < deadline {
        sleep(Duration::from_millis(100));
        let now = sys::fd_count(pid)?;
        if (target > 0 && now <= target) || (target == 0 && now == last) {
            return Ok(now);
        }
        last = now;
    }
    Ok(last)
}

fn load_average() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|text| sys::parse_loadavg(&text))
        .unwrap_or_else(|| "unavailable".to_owned())
}

fn command_output(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map_or_else(
            || "unavailable".to_owned(),
            |out| String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        )
}

fn environment(binary: &Path, site: &BenchSite) -> Vec<(String, String)> {
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| sys::parse_cpu_model(&text))
        .unwrap_or_else(|| command_output("sysctl", &["-n", "machdep.cpu.brand_string"]));
    let cores = std::thread::available_parallelism().map_or(0, std::num::NonZero::get);
    let temp = site.temp_dir().to_string_lossy().into_owned();
    let filesystem = command_output("stat", &["-f", "-c", "%T", &temp]);
    vec![
        (
            "os".to_owned(),
            format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        ),
        ("kernel".to_owned(), command_output("uname", &["-sr"])),
        ("cpu".to_owned(), cpu),
        ("cores".to_owned(), cores.to_string()),
        ("filesystem".to_owned(), filesystem),
        ("rustc".to_owned(), command_output("rustc", &["--version"])),
        (
            "binary".to_owned(),
            command_output(&binary.to_string_lossy(), &["--version"]),
        ),
        ("served_files".to_owned(), site::SERVED_FILES.to_string()),
        ("ignored_files".to_owned(), site::IGNORED_FILES.to_string()),
        ("load_average_start".to_owned(), load_average()),
    ]
}
