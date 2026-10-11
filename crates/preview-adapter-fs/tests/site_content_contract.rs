//! The `SiteContent` contract harness against the real root capability over a temporary
//! directory, and its negative control: an adapter that answers a `Forbidden` refusal with the
//! file's contents whenever the resolved path stays inside the root fails the harness at
//! `in_root_symlink`, the same case as the broken fake in preview-testkit (foundation.md
//! sections 5.1 and 12, phase 2).

use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

use preview_adapter_fs::{RootDir, open_site_root};
use preview_application::{ContentFile, ContentRefusal, Opened, SiteContent};
use preview_domain::SitePath;
use preview_testkit::contracts::{
    LinkTarget, SITE_CONTENT_CASES, SiteContentFactory, SiteEntry, SiteTree,
    check_site_content_contract, site_content_contract,
};
use tempfile::TempDir;

/// `RootDir` over a fresh temporary directory: `<base>/site` is the root and
/// `<base>/outside` holds the tree's outside files. Dropping it removes both.
struct TempSite {
    root: PathBuf,
    dir: RootDir,
    _base: TempDir,
}

fn create_parent(path: &Path) {
    fs::create_dir_all(path.parent().expect("an entry has a parent")).expect("create parents");
}

impl SiteContentFactory for TempSite {
    type Site = Self;

    fn build(tree: &SiteTree) -> Self {
        let base = tempfile::tempdir().expect("tempdir");
        let root = base.path().join("site");
        let outside = base.path().join("outside");
        fs::create_dir(&root).expect("create root");
        fs::create_dir(&outside).expect("create outside");
        for (rel, content) in tree.outside_files() {
            let path = outside.join(rel);
            create_parent(&path);
            fs::write(path, content).expect("write outside file");
        }
        for (rel, entry) in tree.entries() {
            let path = root.join(rel);
            create_parent(&path);
            match entry {
                SiteEntry::Dir => fs::create_dir_all(&path).expect("create dir"),
                SiteEntry::File(content) => fs::write(&path, content).expect("write file"),
                SiteEntry::Symlink(LinkTarget::Inside(target)) => {
                    symlink(root.join(target), &path).expect("create in-root link");
                }
                SiteEntry::Symlink(LinkTarget::Outside(target)) => {
                    symlink(outside.join(target), &path).expect("create outside link");
                }
                SiteEntry::Special => {
                    let status = Command::new("mkfifo")
                        .arg(&path)
                        .status()
                        .expect("run mkfifo");
                    assert!(status.success(), "mkfifo failed");
                }
            }
        }
        let dir = open_site_root(&root).expect("open site root").dir;
        Self {
            root,
            dir,
            _base: base,
        }
    }
}

impl SiteContent for TempSite {
    fn open(&self, path: &SitePath) -> Result<Opened, ContentRefusal> {
        SiteContent::open(&self.dir, path)
    }
}

/// A plain `std::fs::File` as a content handle.
struct StdFile {
    file: File,
    len: u64,
}

impl Read for StdFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

impl ContentFile for StdFile {
    fn len(&self) -> u64 {
        self.len
    }
}

/// The deliberately broken adapter: when `RootDir` refuses a path as `Forbidden`, it falls
/// back to the classic check this project forbids (plan.md section 5): canonicalize the joined
/// path, accept it when it `starts_with` the canonical root and is a regular file, and serve
/// that file's contents. Links leading outside stay refused.
struct ForbiddenToContents(TempSite);

impl SiteContentFactory for ForbiddenToContents {
    type Site = Self;

    fn build(tree: &SiteTree) -> Self {
        Self(TempSite::build(tree))
    }
}

impl ForbiddenToContents {
    fn fallback(&self, path: &SitePath) -> Option<Opened> {
        let joined = path
            .components()
            .iter()
            .fold(self.0.root.clone(), |acc, name| acc.join(name.as_str()));
        let real = fs::canonicalize(joined).ok()?;
        let root = fs::canonicalize(&self.0.root).ok()?;
        if !real.starts_with(root) || !real.is_file() {
            return None;
        }
        let file = File::open(&real).ok()?;
        let len = file.metadata().ok()?.len();
        Some(Opened::File(Box::new(StdFile { file, len })))
    }
}

impl SiteContent for ForbiddenToContents {
    fn open(&self, path: &SitePath) -> Result<Opened, ContentRefusal> {
        match self.0.open(path) {
            Err(ContentRefusal::Forbidden(reason)) => {
                self.fallback(path).ok_or(ContentRefusal::Forbidden(reason))
            }
            other => other,
        }
    }
}

#[test]
fn should_pass_contract_when_site_is_root_dir_over_tempdir() {
    // given
    // TempSite builds each case's tree under a fresh temporary directory.

    // when
    let report = check_site_content_contract::<TempSite>();

    // then
    assert_eq!(report.cases_run(), SITE_CONTENT_CASES);
    report.assert_passed();
    site_content_contract::<TempSite>();
}

#[test]
fn should_fail_contract_when_adapter_maps_forbidden_to_file_contents() {
    // given
    // ForbiddenToContents wraps RootDir and serves in-root link targets it refused.

    // when
    let report = check_site_content_contract::<ForbiddenToContents>();

    // then
    assert_eq!(report.cases_run(), SITE_CONTENT_CASES);
    assert_eq!(report.failed_cases(), ["in_root_symlink"], "{report}");
    let detail = &report.failures()[0].detail;
    assert!(
        detail.contains("open `/mirror/page.html`: expected Refused(Forbidden(Symlink))"),
        "{detail}"
    );
}
