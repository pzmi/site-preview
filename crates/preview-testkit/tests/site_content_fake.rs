//! The `SiteContent` contract harness against the in-memory fake, and its negative control: a
//! fake that follows links which stay inside the root fails the harness at `in_root_symlink`
//! and nowhere else (foundation.md sections 5.1 and 12, phase 2).

use preview_application::{ContentRefusal, Opened, SiteContent};
use preview_domain::{SitePath, decode_request_path};
use preview_testkit::InMemorySite;
use preview_testkit::contracts::{
    LinkTarget, SITE_CONTENT_CASES, SiteContentFactory, SiteEntry, SiteTree,
    check_site_content_contract, site_content_contract,
};

/// The deliberately broken fake: before opening, it rewrites every leading run of components
/// that names a link pointing inside the root to that link's target, as a resolver that
/// "allows harmless in-root links" would. Links leading outside stay refused.
struct BrokenFollowsSymlinks {
    site: InMemorySite,
    links: Vec<(Vec<String>, Vec<String>)>,
}

fn split(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

impl SiteContentFactory for BrokenFollowsSymlinks {
    type Site = Self;

    fn build(tree: &SiteTree) -> Self {
        let links = tree
            .entries()
            .iter()
            .filter_map(|(path, entry)| match entry {
                SiteEntry::Symlink(LinkTarget::Inside(target)) => {
                    Some((split(path), split(target)))
                }
                _ => None,
            })
            .collect();
        Self {
            site: InMemorySite::build(tree),
            links,
        }
    }
}

impl BrokenFollowsSymlinks {
    /// Follows in-root links one hop at a time, a bounded number of hops.
    fn resolve(&self, path: &SitePath) -> Option<SitePath> {
        let mut names: Vec<String> = path
            .components()
            .iter()
            .map(|name| name.as_str().to_owned())
            .collect();
        let mut followed = false;
        for _ in 0..8 {
            let Some((link, target)) = self.links.iter().find(|(link, _)| names.starts_with(link))
            else {
                break;
            };
            names = target.iter().chain(&names[link.len()..]).cloned().collect();
            followed = true;
        }
        followed
            .then(|| decode_request_path(&format!("/{}", names.join("/"))).ok())
            .flatten()
    }
}

impl SiteContent for BrokenFollowsSymlinks {
    fn open(&self, path: &SitePath) -> Result<Opened, ContentRefusal> {
        match self.resolve(path) {
            Some(resolved) => self.site.open(&resolved),
            None => self.site.open(path),
        }
    }
}

#[test]
fn should_pass_contract_when_site_is_in_memory_fake() {
    // given
    // InMemorySite builds itself from the contract tree.

    // when
    let report = check_site_content_contract::<InMemorySite>();

    // then
    assert_eq!(report.cases_run(), SITE_CONTENT_CASES);
    report.assert_passed();
    site_content_contract::<InMemorySite>();
}

#[test]
fn should_fail_contract_when_fake_follows_in_root_symlinks() {
    // given
    // BrokenFollowsSymlinks wraps the fake and follows links that stay inside the root.

    // when
    let report = check_site_content_contract::<BrokenFollowsSymlinks>();

    // then
    assert_eq!(report.cases_run(), SITE_CONTENT_CASES);
    assert_eq!(report.failed_cases(), ["in_root_symlink"], "{report}");
    let detail = &report.failures()[0].detail;
    assert!(
        detail.contains("open `/mirror/page.html`: expected Refused(Forbidden(Symlink))"),
        "{detail}"
    );
}

#[test]
fn should_panic_with_case_name_when_contract_is_asserted_on_broken_fake() {
    // given
    // The broken fake fails the harness; the asserting entry point must say where.

    // when
    let outcome = std::panic::catch_unwind(site_content_contract::<BrokenFollowsSymlinks>);

    // then
    let payload = outcome.expect_err("the harness must panic on a broken implementation");
    let message = payload
        .downcast_ref::<String>()
        .expect("the harness panics with a formatted report");
    assert!(message.contains("1 of 15 cases failed"), "{message}");
    assert!(message.contains("case `in_root_symlink`"), "{message}");
}
