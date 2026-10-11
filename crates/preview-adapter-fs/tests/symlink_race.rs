//! The root capability under a concurrent symlink swap (plan.md section 5.3, last paragraph;
//! ADR 0004).
//!
//! One thread atomically exchanges (`renameat2(RENAME_EXCHANGE)` on Linux, `renameatx_np`
//! with `RENAME_SWAP` on macOS) the site's real directory `d/` with a symbolic link to a
//! directory outside the root, so `d` always exists and is, at any instant, either the real
//! directory or the link. The test thread keeps opening `d/secret.txt` meanwhile. Outside the
//! root, `secret.txt` holds a sentinel; inside, it holds other bytes.
//!
//! Both loops are bounded by iteration counts, never by time: the opener makes at least
//! [`MIN_ATTEMPTS`] and at most [`MAX_ATTEMPTS`] opens, stopping early once it has seen what
//! the test needs, and the swapper stops when the opener does (or after [`MAX_SWAPS`]).
//! Nothing sleeps or waits on a clock; a barrier only starts both loops together.
//!
//! The naive variant runs the same race against `std::fs` path resolution, which follows the
//! link, to show the race is real: it must observe the sentinel. Without that, a capability
//! test that never saw a link mid-walk would pass vacuously; the `forbidden` count of the
//! capability test guards against the same.

use std::fs;
use std::io::Read;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::Barrier;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use preview_adapter_fs::{Opened, open_site_root};
use preview_application::{ContentRefusal, ForbiddenReason};
use preview_domain::{SitePath, decode_request_path};
use rustix::fs::{CWD, RenameFlags, renameat_with};
use tempfile::TempDir;

/// Bytes of the file outside the root: returning them is an escape.
const SENTINEL: &[u8] = b"synthetic-sentinel: outside the site root";
/// Bytes of the file inside the root, under the same name.
const INSIDE: &[u8] = b"inside the site root";

/// Opens the opener always makes, so a passing run has exercised the race for a while even
/// when both outcomes show up at once.
const MIN_ATTEMPTS: u32 = 20_000;
/// The opener's hard bound.
const MAX_ATTEMPTS: u32 = 400_000;
/// The swapper's hard bound, a safety net: it normally stops when the opener does.
const MAX_SWAPS: u32 = 10_000_000;

/// Raises the swapper's stop flag when dropped.
struct StopOnDrop<'a>(&'a AtomicBool);

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// What one open of `d/secret.txt` produced.
#[derive(Debug)]
enum Outcome {
    /// The file inside the root.
    Inside,
    /// The sentinel from outside the root.
    Escaped,
    /// Refused because a component was a symbolic link.
    Forbidden,
    /// Anything else, kept for the failure message.
    Unexpected(String),
}

/// Counts of each outcome over one race.
#[derive(Debug, Default)]
struct Tally {
    attempts: u32,
    inside: u32,
    escaped: u32,
    forbidden: u32,
    unexpected: Vec<String>,
}

impl Tally {
    fn record(&mut self, outcome: Outcome) {
        self.attempts += 1;
        match outcome {
            Outcome::Inside => self.inside += 1,
            Outcome::Escaped => self.escaped += 1,
            Outcome::Forbidden => self.forbidden += 1,
            Outcome::Unexpected(what) => {
                if self.unexpected.len() < 8 {
                    self.unexpected.push(what);
                }
            }
        }
    }
}

/// A site root with `d/secret.txt` inside, a sibling `outside/secret.txt` holding the
/// sentinel, and `alt`, a link inside the root to `outside`, which the swapper exchanges
/// with `d`.
struct Fixture {
    _base: TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let base = tempfile::tempdir().expect("tempdir");
        let root = base.path().join("site");
        let outside = base.path().join("outside");
        fs::create_dir_all(root.join("d")).expect("create d");
        fs::create_dir(&outside).expect("create outside");
        fs::write(root.join("d").join("secret.txt"), INSIDE).expect("write inside file");
        fs::write(outside.join("secret.txt"), SENTINEL).expect("write sentinel");
        symlink(&outside, root.join("alt")).expect("create link");
        Self { _base: base, root }
    }

    /// Runs `probe` against the swapping tree until `enough` holds (after at least
    /// [`MIN_ATTEMPTS`]) or [`MAX_ATTEMPTS`] is reached, and returns what it saw.
    fn race(&self, mut probe: impl FnMut() -> Outcome, enough: impl Fn(&Tally) -> bool) -> Tally {
        let stop = AtomicBool::new(false);
        let start = Barrier::new(2);
        let d = self.root.join("d");
        let alt = self.root.join("alt");
        let mut tally = Tally::default();
        let swaps = thread::scope(|scope| {
            let swapper = scope.spawn(|| {
                start.wait();
                let mut swaps = 0_u32;
                while !stop.load(Ordering::Relaxed) && swaps < MAX_SWAPS {
                    renameat_with(CWD, &d, CWD, &alt, RenameFlags::EXCHANGE)
                        .expect("exchange d and alt");
                    swaps += 1;
                }
                swaps
            });
            // Set on every exit, a panicking probe included, so the swapper never outlives
            // the opener by more than one exchange.
            let stop_swapper = StopOnDrop(&stop);
            start.wait();
            while tally.attempts < MAX_ATTEMPTS
                && (tally.attempts < MIN_ATTEMPTS || !enough(&tally))
            {
                tally.record(probe());
            }
            drop(stop_swapper);
            swapper.join().expect("swapper thread")
        });
        assert!(swaps > 0, "the swapper never ran: {tally:?}");
        tally
    }
}

fn classify(body: &[u8]) -> Outcome {
    match body {
        SENTINEL => Outcome::Escaped,
        INSIDE => Outcome::Inside,
        other => Outcome::Unexpected(format!("{} unknown bytes", other.len())),
    }
}

fn site_path(raw: &str) -> SitePath {
    decode_request_path(raw).expect("valid request path")
}

/// The capability under test: [`preview_adapter_fs::RootDir::open`], reading what it opened.
fn open_with_capability(root: &Path) -> impl FnMut() -> Outcome {
    let dir = open_site_root(root).expect("open site root").dir;
    let path = site_path("/d/secret.txt");
    move || match dir.open(&path) {
        Ok(Opened::File(file)) => {
            let mut body = Vec::new();
            fs::File::from(file.into_fd())
                .read_to_end(&mut body)
                .expect("read opened file");
            classify(&body)
        }
        Ok(Opened::Directory(_)) => Outcome::Unexpected("a directory".to_owned()),
        Err(ContentRefusal::Forbidden(ForbiddenReason::Symlink)) => Outcome::Forbidden,
        Err(other) => Outcome::Unexpected(format!("{other:?}")),
    }
}

/// The unsafe baseline: resolve the joined path with ambient authority, following links.
fn open_naively(root: &Path) -> impl FnMut() -> Outcome {
    let joined = root.join("d/secret.txt");
    move || match fs::File::open(&joined) {
        Ok(mut file) => {
            let mut body = Vec::new();
            file.read_to_end(&mut body).expect("read file");
            classify(&body)
        }
        Err(error) => Outcome::Unexpected(format!("{:?}", error.kind())),
    }
}

mod symlink_race {
    use super::{Fixture, open_naively, open_with_capability};

    #[test]
    fn should_never_return_outside_bytes_when_directory_is_swapped_for_symlink() {
        // given
        let site = Fixture::new();
        let probe = open_with_capability(&site.root);

        // when
        let tally = site.race(probe, |t| t.inside > 0 && t.forbidden > 0);

        // then
        assert_eq!(
            tally.escaped, 0,
            "the capability returned the sentinel: {tally:?}"
        );
        assert!(
            tally.unexpected.is_empty(),
            "unexpected outcomes: {tally:?}"
        );
        assert!(tally.inside > 0, "never opened the file inside: {tally:?}");
        assert!(
            tally.forbidden > 0,
            "never met the link mid-walk: {tally:?}"
        );
    }

    #[test]
    fn should_return_outside_bytes_when_path_is_opened_naively_during_swap() {
        // given
        let site = Fixture::new();
        let probe = open_naively(&site.root);

        // when
        let tally = site.race(probe, |t| t.escaped > 0);

        // then
        assert!(tally.escaped > 0, "the naive open never escaped: {tally:?}");
    }
}
