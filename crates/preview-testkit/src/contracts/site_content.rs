//! The `SiteContent` contract harness: every implementation of the port, the in-memory fake
//! and the real root capability alike, is run through the same named cases (ADR 0038, the
//! contract on `preview_application::SiteContent`).
//!
//! A test supplies a [`SiteContentFactory`] that builds its implementation from a
//! [`SiteTree`], then calls [`site_content_contract`] (panics with the report) or
//! [`check_site_content_contract`] (returns it, for a negative control).
//!
//! Not covered here, because it is not portable: a component the operating system denies
//! (`Forbidden(PermissionDenied)`). A superuser holding its DAC capabilities, as in a
//! container, reads a mode-000 directory, so a real adapter cannot be made to refuse; the
//! fake's own unit tests and the adapter's tests cover that variant.

use core::fmt;
use std::io::Read;
use std::thread;

use preview_application::{ContentFile, ContentRefusal, ForbiddenReason, Opened, SiteContent};
use preview_domain::decode_request_path;

use super::{Case, ContractReport, panic_text, run_cases};

/// Where a symbolic link in a [`SiteTree`] points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// A `/`-separated path relative to the site root: a link that stays inside the site.
    Inside(String),
    /// A `/`-separated path relative to the outside directory, a sibling of the root that
    /// holds the tree's [`outside_files`](SiteTree::outside_files).
    Outside(String),
}

/// One entry of a [`SiteTree`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SiteEntry {
    /// An empty directory.
    Dir,
    /// A regular file with these bytes.
    File(Vec<u8>),
    /// A symbolic link. A factory that cannot follow links (the in-memory fake) may ignore the
    /// target; a real one creates the link pointing there.
    Symlink(LinkTarget),
    /// Neither a regular file nor a directory: a FIFO on a real filesystem.
    Special,
}

/// A site described declaratively: entries beneath the root, plus files outside it that links
/// may point to.
///
/// Paths are `/`-separated and relative; a factory creates missing parent directories. Entries
/// are listed in the order they were added.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SiteTree {
    entries: Vec<(String, SiteEntry)>,
    outside: Vec<(String, Vec<u8>)>,
}

impl SiteTree {
    /// An empty site: the root directory only.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a regular file.
    #[must_use]
    pub fn file(self, path: &str, content: &[u8]) -> Self {
        self.entry(path, SiteEntry::File(content.to_vec()))
    }

    /// Adds an empty directory.
    #[must_use]
    pub fn dir(self, path: &str) -> Self {
        self.entry(path, SiteEntry::Dir)
    }

    /// Adds a symbolic link to `target`.
    #[must_use]
    pub fn symlink(self, path: &str, target: LinkTarget) -> Self {
        self.entry(path, SiteEntry::Symlink(target))
    }

    /// Adds a special file (a FIFO).
    #[must_use]
    pub fn special(self, path: &str) -> Self {
        self.entry(path, SiteEntry::Special)
    }

    /// Adds a file outside the root, for a [`LinkTarget::Outside`] link to point at.
    #[must_use]
    pub fn outside_file(mut self, path: &str, content: &[u8]) -> Self {
        self.outside.push((path.to_owned(), content.to_vec()));
        self
    }

    /// The entries beneath the root, in the order they were added.
    #[must_use]
    pub fn entries(&self) -> &[(String, SiteEntry)] {
        &self.entries
    }

    /// The files outside the root, in the order they were added.
    #[must_use]
    pub fn outside_files(&self) -> &[(String, Vec<u8>)] {
        &self.outside
    }

    fn entry(mut self, path: &str, entry: SiteEntry) -> Self {
        self.entries.push((path.to_owned(), entry));
        self
    }
}

/// Builds the implementation under test from a [`SiteTree`].
///
/// The built value owns whatever keeps the site alive (a temporary directory, say): the
/// harness drops it to check that opened handles outlive it.
pub trait SiteContentFactory {
    /// The implementation under test.
    type Site: SiteContent;

    /// Builds a fresh site holding exactly `tree`.
    fn build(tree: &SiteTree) -> Self::Site;
}

/// The names of the harness's cases, in run order.
pub const SITE_CONTENT_CASES: &[&str] = &[
    "regular_file",
    "missing",
    "nested_index",
    "trailing_slash",
    "missing_index",
    "index_refused",
    "hidden",
    "symlink_final",
    "symlink_middle",
    "in_root_symlink",
    "non_regular",
    "name_too_long",
    "refusal_text_without_paths",
    "independent_handles",
    "concurrent_calls",
];

/// Runs every case against `F` and panics with the full report if any failed.
///
/// # Panics
///
/// When the implementation breaks a case of the contract.
pub fn site_content_contract<F: SiteContentFactory>() {
    check_site_content_contract::<F>().assert_passed();
}

/// Runs every case against `F` and returns the report; a panic inside a case (in the factory
/// or the implementation) is that case's failure.
#[must_use]
pub fn check_site_content_contract<F: SiteContentFactory>() -> ContractReport {
    let cases: [(&'static str, Case); 15] = [
        ("regular_file", regular_file::<F>),
        ("missing", missing::<F>),
        ("nested_index", nested_index::<F>),
        ("trailing_slash", trailing_slash::<F>),
        ("missing_index", missing_index::<F>),
        ("index_refused", index_refused::<F>),
        ("hidden", hidden::<F>),
        ("symlink_final", symlink_final::<F>),
        ("symlink_middle", symlink_middle::<F>),
        ("in_root_symlink", in_root_symlink::<F>),
        ("non_regular", non_regular::<F>),
        ("name_too_long", name_too_long::<F>),
        (
            "refusal_text_without_paths",
            refusal_text_without_paths::<F>,
        ),
        ("independent_handles", independent_handles::<F>),
        ("concurrent_calls", concurrent_calls::<F>),
    ];
    run_cases(&cases)
}

/// Bytes a link outside the root leads to; seeing them anywhere is a confinement breach.
const SECRET: &[u8] = b"outside secret";
const SYMLINK: ContentRefusal = ContentRefusal::Forbidden(ForbiddenReason::Symlink);
const NOT_REGULAR: ContentRefusal = ContentRefusal::Forbidden(ForbiddenReason::NotRegular);
/// `NAME_MAX` on the platforms the adapter supports.
const NAME_MAX: usize = 255;

/// What an `open` did, with the bytes of any handle read out.
#[derive(Clone, PartialEq, Eq)]
enum Observed {
    File(Vec<u8>),
    Directory { index: Option<Vec<u8>> },
    Refused(ContentRefusal),
}

impl Observed {
    fn file(bytes: &[u8]) -> Self {
        Self::File(bytes.to_vec())
    }

    fn index(bytes: &[u8]) -> Self {
        Self::Directory {
            index: Some(bytes.to_vec()),
        }
    }

    const NO_INDEX: Self = Self::Directory { index: None };
}

impl fmt::Debug for Observed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(bytes) => write!(f, "File({:?})", String::from_utf8_lossy(bytes)),
            Self::Directory { index: None } => f.write_str("Directory { index: None }"),
            Self::Directory { index: Some(bytes) } => write!(
                f,
                "Directory {{ index: Some({:?}) }}",
                String::from_utf8_lossy(bytes)
            ),
            Self::Refused(refusal) => write!(f, "Refused({refusal:?})"),
        }
    }
}

/// Reads a handle to its end and checks it against its own `len`.
fn read_handle(mut file: Box<dyn ContentFile>) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("reading the handle failed: {error}"))?;
    let read = u64::try_from(bytes.len()).map_err(|_| "handle larger than u64".to_owned())?;
    if read != file.len() {
        return Err(format!(
            "handle reported len {} but yielded {read} bytes",
            file.len()
        ));
    }
    Ok(bytes)
}

fn observe_opened(opened: Opened) -> Result<Observed, String> {
    Ok(match opened {
        Opened::File(file) => Observed::File(read_handle(file)?),
        Opened::Directory { index: None } => Observed::NO_INDEX,
        Opened::Directory { index: Some(file) } => Observed::Directory {
            index: Some(read_handle(file)?),
        },
    })
}

fn open_raw(site: &dyn SiteContent, raw: &str) -> Result<Result<Opened, ContentRefusal>, String> {
    let path = decode_request_path(raw)
        .map_err(|refusal| format!("harness bug: `{raw}` does not decode: {refusal:?}"))?;
    Ok(site.open(&path))
}

fn observe(site: &dyn SiteContent, raw: &str) -> Result<Observed, String> {
    match open_raw(site, raw)? {
        Ok(opened) => observe_opened(opened),
        Err(refusal) => Ok(Observed::Refused(refusal)),
    }
}

/// Collects every failed check of one case, so one run shows all of them.
#[derive(Default)]
struct Checks(Vec<String>);

impl Checks {
    fn expect(&mut self, site: &dyn SiteContent, raw: &str, want: &Observed) {
        match observe(site, raw) {
            Ok(got) if got == *want => {}
            Ok(got) => self.fail(format!(
                "open `{}`: expected {want:?}, got {got:?}",
                shorten(raw)
            )),
            Err(problem) => self.fail(format!("open `{}`: {problem}", shorten(raw))),
        }
    }

    fn fail(&mut self, line: String) {
        self.0.push(line);
    }

    fn finish(self) -> Result<(), String> {
        if self.0.is_empty() {
            return Ok(());
        }
        Err(self
            .0
            .iter()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

/// Keeps a failure line readable when the request carries an over-long name.
fn shorten(raw: &str) -> String {
    if raw.len() <= 80 {
        return raw.to_owned();
    }
    let head: String = raw.chars().take(40).collect();
    format!("{head}... ({} bytes)", raw.len())
}

/// A regular file opens as a file and yields its bytes, wherever it sits.
fn regular_file<F: SiteContentFactory>() -> Result<(), String> {
    let site = F::build(
        &SiteTree::new()
            .file("page.html", b"<p>page</p>")
            .file("css/deep/site.css", b"body{}")
            .file("empty.txt", b""),
    );
    let mut checks = Checks::default();
    checks.expect(&site, "/page.html", &Observed::file(b"<p>page</p>"));
    checks.expect(&site, "/css/deep/site.css", &Observed::file(b"body{}"));
    checks.expect(&site, "/empty.txt", &Observed::file(b""));
    checks.finish()
}

/// A missing component, final or intermediate, is `NotFound`.
fn missing<F: SiteContentFactory>() -> Result<(), String> {
    let site = F::build(&SiteTree::new().file("a/b.html", b"b"));
    let mut checks = Checks::default();
    let not_found = Observed::Refused(ContentRefusal::NotFound);
    checks.expect(&site, "/a/missing.html", &not_found);
    checks.expect(&site, "/nope/b.html", &not_found);
    checks.expect(&site, "/nope/", &not_found);
    checks.finish()
}

/// A directory opens its own `index.html`, at the root and at any depth.
fn nested_index<F: SiteContentFactory>() -> Result<(), String> {
    let site = F::build(
        &SiteTree::new()
            .file("index.html", b"home")
            .file("docs/index.html", b"docs")
            .file("docs/guide/index.html", b"guide")
            .file("docs/guide/other.html", b"other"),
    );
    let mut checks = Checks::default();
    checks.expect(&site, "/", &Observed::index(b"home"));
    checks.expect(&site, "/docs", &Observed::index(b"docs"));
    checks.expect(&site, "/docs/guide", &Observed::index(b"guide"));
    checks.finish()
}

/// `a/b` and `a/b/` open the same object, file or directory.
fn trailing_slash<F: SiteContentFactory>() -> Result<(), String> {
    let site = F::build(
        &SiteTree::new()
            .file("docs/index.html", b"docs")
            .file("page.html", b"page")
            .dir("bare"),
    );
    let mut checks = Checks::default();
    checks.expect(&site, "/docs/", &Observed::index(b"docs"));
    checks.expect(&site, "/docs", &Observed::index(b"docs"));
    checks.expect(&site, "/page.html/", &Observed::file(b"page"));
    checks.expect(&site, "/bare/", &Observed::NO_INDEX);
    checks.finish()
}

/// A directory without `index.html` opens with no index; no other name counts as one.
fn missing_index<F: SiteContentFactory>() -> Result<(), String> {
    let tree = SiteTree::new()
        .dir("empty")
        .file("other/index.htm", b"htm")
        .file("other/readme.txt", b"readme");
    // `INDEX.HTML` is another name only on a case-sensitive file system; on the default
    // macOS volume it is `index.html` itself (eligibility.rs), so the case does not apply.
    let upper = (!cfg!(target_os = "macos")).then_some("other/INDEX.HTML");
    let tree = upper
        .into_iter()
        .fold(tree, |tree, path| tree.file(path, b"upper"));
    let site = F::build(&tree);
    let bare = F::build(&SiteTree::new().file("a.txt", b"a"));
    let mut checks = Checks::default();
    checks.expect(&site, "/empty", &Observed::NO_INDEX);
    checks.expect(&site, "/other", &Observed::NO_INDEX);
    checks.expect(&bare, "/", &Observed::NO_INDEX);
    checks.finish()
}

/// An `index.html` that exists but is refused makes the whole call that refusal.
fn index_refused<F: SiteContentFactory>() -> Result<(), String> {
    let site = F::build(
        &SiteTree::new()
            .outside_file("index.html", SECRET)
            .symlink(
                "linked/index.html",
                LinkTarget::Outside("index.html".into()),
            )
            .special("fifo/index.html")
            .dir("nested/index.html")
            .file("nested/index.html/index.html", b"inner"),
    );
    let mut checks = Checks::default();
    checks.expect(&site, "/linked", &Observed::Refused(SYMLINK));
    checks.expect(&site, "/linked/", &Observed::Refused(SYMLINK));
    checks.expect(&site, "/fifo", &Observed::Refused(NOT_REGULAR));
    checks.expect(&site, "/nested", &Observed::Refused(NOT_REGULAR));
    checks.finish()
}

/// No eligibility rule applies: hidden and excluded names open like any other.
fn hidden<F: SiteContentFactory>() -> Result<(), String> {
    let site = F::build(
        &SiteTree::new()
            .file(".git/config", b"config")
            .file(".env", b"env")
            .file("node_modules/x.js", b"js")
            .file("draft.html~", b"backup"),
    );
    let mut checks = Checks::default();
    checks.expect(&site, "/.git/config", &Observed::file(b"config"));
    checks.expect(&site, "/.env", &Observed::file(b"env"));
    checks.expect(&site, "/node_modules/x.js", &Observed::file(b"js"));
    checks.expect(&site, "/draft.html~", &Observed::file(b"backup"));
    checks.finish()
}

/// A link as the final component is refused, whether it leads to a file or a directory.
fn symlink_final<F: SiteContentFactory>() -> Result<(), String> {
    let site = F::build(
        &SiteTree::new()
            .outside_file("secret.html", SECRET)
            .outside_file("dir/index.html", SECRET)
            .symlink("leak.html", LinkTarget::Outside("secret.html".into()))
            .symlink("leakdir", LinkTarget::Outside("dir".into()))
            .symlink("dangling", LinkTarget::Outside("nothing-here".into())),
    );
    let mut checks = Checks::default();
    checks.expect(&site, "/leak.html", &Observed::Refused(SYMLINK));
    checks.expect(&site, "/leakdir", &Observed::Refused(SYMLINK));
    checks.expect(&site, "/leakdir/", &Observed::Refused(SYMLINK));
    checks.expect(&site, "/dangling", &Observed::Refused(SYMLINK));
    checks.finish()
}

/// A link as an intermediate component is refused as `Symlink`; a regular file there is
/// `NotFound` (a path through a file names nothing), so the refusal does not reveal the file.
fn symlink_middle<F: SiteContentFactory>() -> Result<(), String> {
    let site = F::build(
        &SiteTree::new()
            .outside_file("dir/secret.html", SECRET)
            .symlink("out", LinkTarget::Outside("dir".into()))
            .file("page.html", b"page"),
    );
    let mut checks = Checks::default();
    checks.expect(&site, "/out/secret.html", &Observed::Refused(SYMLINK));
    checks.expect(&site, "/out/missing.html", &Observed::Refused(SYMLINK));
    checks.expect(
        &site,
        "/page.html/x",
        &Observed::Refused(ContentRefusal::NotFound),
    );
    checks.expect(
        &site,
        "/page.html/x/y.html",
        &Observed::Refused(ContentRefusal::NotFound),
    );
    checks.finish()
}

/// A link that stays inside the root is refused too: never followed, at any component, nor
/// as a directory's index. The targets themselves open, so the refusal is the link's.
fn in_root_symlink<F: SiteContentFactory>() -> Result<(), String> {
    let site = F::build(
        &SiteTree::new()
            .file("index.html", b"home")
            .file("real/page.html", b"real page")
            .file("real/index.html", b"real index")
            .symlink("alias.html", LinkTarget::Inside("real/page.html".into()))
            .symlink("mirror", LinkTarget::Inside("real".into()))
            .symlink(
                "aliased/index.html",
                LinkTarget::Inside("index.html".into()),
            ),
    );
    let mut checks = Checks::default();
    checks.expect(&site, "/real/page.html", &Observed::file(b"real page"));
    checks.expect(&site, "/real", &Observed::index(b"real index"));
    checks.expect(&site, "/alias.html", &Observed::Refused(SYMLINK));
    checks.expect(&site, "/mirror/page.html", &Observed::Refused(SYMLINK));
    checks.expect(&site, "/mirror", &Observed::Refused(SYMLINK));
    checks.expect(&site, "/aliased", &Observed::Refused(SYMLINK));
    checks.finish()
}

/// Anything but a regular file or a directory is `NotRegular`, and the call returns at once
/// (a FIFO without a writer must not block the open). A path through one names nothing.
fn non_regular<F: SiteContentFactory>() -> Result<(), String> {
    let site = F::build(&SiteTree::new().special("pipe").special("sub/pipe.html"));
    let mut checks = Checks::default();
    checks.expect(&site, "/pipe", &Observed::Refused(NOT_REGULAR));
    checks.expect(&site, "/sub/pipe.html", &Observed::Refused(NOT_REGULAR));
    checks.expect(
        &site,
        "/pipe/x",
        &Observed::Refused(ContentRefusal::NotFound),
    );
    checks.finish()
}

/// A component longer than `NAME_MAX` bytes names nothing that can exist, so it is
/// `NotFound`, like a missing name of exactly `NAME_MAX` bytes: a request must not turn into
/// a server failure (`Io`) by being long. Final, intermediate or after a missing component,
/// the answer is the same.
fn name_too_long<F: SiteContentFactory>() -> Result<(), String> {
    let site = F::build(&SiteTree::new().file("a/b.html", b"b"));
    let at_max = "n".repeat(NAME_MAX);
    let over = "n".repeat(NAME_MAX + 1);
    let too_long = Observed::Refused(ContentRefusal::NotFound);
    let not_found = Observed::Refused(ContentRefusal::NotFound);
    let mut checks = Checks::default();
    checks.expect(&site, &format!("/{over}"), &too_long);
    checks.expect(&site, &format!("/a/{over}"), &too_long);
    checks.expect(&site, &format!("/{over}/b.html"), &too_long);
    checks.expect(&site, &format!("/{at_max}"), &not_found);
    checks.expect(&site, &format!("/missing/{over}"), &not_found);
    checks.finish()
}

/// Every refusal renders as a category only: no path, no file name, no OS text.
fn refusal_text_without_paths<F: SiteContentFactory>() -> Result<(), String> {
    let site = F::build(
        &SiteTree::new()
            .outside_file("zq-secret.html", SECRET)
            .symlink("zq-link.html", LinkTarget::Outside("zq-secret.html".into()))
            .special("zq-pipe")
            .file("zq-index/index.html", b"i"),
    );
    let long = format!("/{}", "zq".repeat(NAME_MAX));
    let requests = [
        "/zq-missing.html",
        "/zq-link.html",
        "/zq-pipe",
        "/zq-pipe/zq-x",
        "/zq-index/index.html/zq-x",
        long.as_str(),
    ];
    let mut checks = Checks::default();
    for raw in requests {
        match open_raw(&site, raw) {
            Err(problem) => checks.fail(problem),
            Ok(Ok(opened)) => checks.fail(format!(
                "open `{}`: expected a refusal, got {opened:?}",
                shorten(raw)
            )),
            Ok(Err(refusal)) => {
                let text = format!("{refusal} {refusal:?}");
                if text.contains('/') || text.contains('\\') || text.contains("zq") {
                    checks.fail(format!(
                        "open `{}`: refusal text carries a location: {text}",
                        shorten(raw)
                    ));
                }
            }
        }
    }
    checks.finish()
}

/// Handles are independent of each other and of the store: two opens of one file each read
/// from the start, and both outlive the store that opened them.
fn independent_handles<F: SiteContentFactory>() -> Result<(), String> {
    let site = F::build(
        &SiteTree::new()
            .file("a.txt", b"kept bytes")
            .file("d/index.html", b"kept index"),
    );
    let mut checks = Checks::default();
    let opens = ["/a.txt", "/a.txt", "/d"].map(|raw| open_raw(&site, raw));
    drop(site);
    let want = [
        Observed::file(b"kept bytes"),
        Observed::file(b"kept bytes"),
        Observed::index(b"kept index"),
    ];
    for (opened, want) in opens.into_iter().zip(want) {
        let got = match opened {
            Err(problem) => Err(problem),
            Ok(Err(refusal)) => Ok(Observed::Refused(refusal)),
            Ok(Ok(opened)) => observe_opened(opened),
        };
        match got {
            Ok(got) if got == want => {}
            Ok(got) => checks.fail(format!(
                "after the store was dropped: expected {want:?}, got {got:?}"
            )),
            Err(problem) => checks.fail(format!("after the store was dropped: {problem}")),
        }
    }
    checks.finish()
}

/// `open` may be called from many threads at once and each call stands alone.
fn concurrent_calls<F: SiteContentFactory>() -> Result<(), String> {
    const THREADS: usize = 4;
    const ROUNDS: usize = 16;
    let site = F::build(
        &SiteTree::new()
            .file("a.txt", b"one")
            .file("b/index.html", b"two"),
    );
    let site: &dyn SiteContent = &site;
    let expected = [
        ("/a.txt", Observed::file(b"one")),
        ("/b/", Observed::index(b"two")),
        ("/missing", Observed::Refused(ContentRefusal::NotFound)),
    ];
    let failures: Vec<String> = thread::scope(|scope| {
        let workers: Vec<_> = (0..THREADS)
            .map(|_| {
                scope.spawn(|| {
                    let mut checks = Checks::default();
                    for _ in 0..ROUNDS {
                        for (raw, want) in &expected {
                            checks.expect(site, raw, want);
                        }
                    }
                    checks.0
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| {
                worker.join().unwrap_or_else(|payload| {
                    vec![format!("a thread panicked: {}", panic_text(&*payload))]
                })
            })
            .collect()
    });
    let mut failures = failures;
    failures.sort();
    failures.dedup();
    Checks(failures).finish()
}
