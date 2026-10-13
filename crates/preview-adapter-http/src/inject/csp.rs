//! Whether a page's `<meta http-equiv="Content-Security-Policy">` would stop the injected
//! client from loading or connecting (plan.md §6 step 8).
//!
//! This only reads. The policy is never altered, no header is added, and the page's bytes keep
//! the meta element exactly as written (pinned by `should_preserve_meta_csp_bytes_when_injecting`
//! in `tests/inject.rs`). Static files carry no CSP header from this server, so a page's meta
//! element is the only policy visible here; a violation that depends on the real origin, a
//! response header from elsewhere or a host-source match is discoverable only in the browser
//! console, and is not reported.
//!
//! Two requests are modelled, both same-origin: loading `/__site_preview/client.js` as a
//! `<script src>`, and the `EventSource` connection to `/__site_preview/events`.
//!
//! - Script: the first present of `script-src-elem`, `script-src`, `default-src` governs.
//! - Connect: the first present of `connect-src`, `default-src` governs.
//! - A list allows the request when it holds `'self'`, `*` or `http:`. `'none'`, an empty list,
//!   and a list of only nonces, hashes, `'unsafe-*'` keywords or other schemes (`https:`,
//!   `data:`, `blob:`) do not: the injected tag has no nonce or hash, and this server speaks
//!   plain `http`, which an `https:` source never matches. `'strict-dynamic'`
//!   makes a script list ignore `'self'`, so it blocks too.
//! - A host-source (`example.com`, `http://127.0.0.1:8080`) may or may not match this server;
//!   without knowing the origin it counts as allowing, so there is no warning (a false
//!   negative, never a false alarm).
//! - Several meta elements, and commas inside one `content`, are several policies; each is
//!   enforced, so any one that blocks is a warning.
//! - A directive repeated inside one policy counts once, the first.

use core::fmt;

/// The warning code (plan.md §6 step 8).
pub(crate) const CSP_BLOCKS_LIVE_RELOAD: &str =
    preview_domain::warning_code::CSP_BLOCKS_LIVE_RELOAD;

/// What a page's policy blocks: the effective directive that stops each request, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CspWarning {
    /// The directive that blocks loading the client script.
    pub script: Option<&'static str>,
    /// The directive that blocks the reload connection.
    pub connect: Option<&'static str>,
}

impl fmt::Display for CspWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("its Content-Security-Policy meta tag blocks ")?;
        match (self.script, self.connect) {
            (Some(script), Some(connect)) => write!(
                f,
                "the reload script ({script}) and the reload connection ({connect})"
            ),
            (Some(script), None) => write!(f, "the reload script ({script})"),
            (None, Some(connect)) => write!(f, "the reload connection ({connect})"),
            (None, None) => f.write_str("nothing"),
        }?;
        f.write_str("; the page is served as written and will not live reload")
    }
}

/// Checks the `content` values of a page's CSP meta elements, in document order.
///
/// Pinned by `should_warn_when_script_src_excludes_self`,
/// `should_warn_when_default_src_none_and_no_connect_src`, `should_not_warn_when_self_allowed`
/// and the other tests below.
#[must_use]
pub fn detect<S: AsRef<str>>(policies: &[S]) -> Option<CspWarning> {
    let mut script = None;
    let mut connect = None;
    for content in policies {
        for policy in content.as_ref().split(',') {
            let directives = Directives::parse(policy);
            script = script.or_else(|| {
                directives.blocked(&["script-src-elem", "script-src", "default-src"], true)
            });
            connect =
                connect.or_else(|| directives.blocked(&["connect-src", "default-src"], false));
        }
    }
    (script.is_some() || connect.is_some()).then_some(CspWarning { script, connect })
}

struct Directives<'a>(Vec<(String, Vec<&'a str>)>);

impl<'a> Directives<'a> {
    fn parse(policy: &'a str) -> Self {
        const WS: [char; 5] = ['\t', '\n', '\u{c}', '\r', ' '];
        let mut parsed: Vec<(String, Vec<&str>)> = Vec::new();
        for directive in policy.split(';') {
            let mut tokens = directive.split(WS).filter(|token| !token.is_empty());
            let Some(name) = tokens.next() else { continue };
            let name = name.to_ascii_lowercase();
            if parsed.iter().all(|(seen, _)| *seen != name) {
                parsed.push((name, tokens.collect()));
            }
        }
        Self(parsed)
    }

    /// The name of the first of `chain` this policy has, when its list refuses a same-origin
    /// request; `None` when it allows it or has none of them.
    fn blocked(&self, chain: &[&'static str], script: bool) -> Option<&'static str> {
        let (name, sources) = chain.iter().find_map(|name| {
            self.0
                .iter()
                .find(|(present, _)| present == name)
                .map(|(_, sources)| (*name, sources))
        })?;
        (!allows_same_origin(sources, script)).then_some(name)
    }
}

fn allows_same_origin(sources: &[&str], script: bool) -> bool {
    // 'none' counts only on its own; with other sources a browser ignores it.
    if sources.is_empty() || sources.iter().eq(["'none'"].iter()) {
        return false;
    }
    let lower: Vec<String> = sources.iter().map(|s| s.to_ascii_lowercase()).collect();
    if script && lower.iter().any(|s| s == "'strict-dynamic'") {
        return false;
    }
    lower.iter().any(|source| match source.as_str() {
        "'self'" | "*" | "http:" => true,
        keyword if keyword.starts_with('\'') => false,
        other => !other.ends_with(':'),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(policy: &str) -> Option<&'static str> {
        detect(&[policy]).and_then(|w| w.script)
    }

    fn connect(policy: &str) -> Option<&'static str> {
        detect(&[policy]).and_then(|w| w.connect)
    }

    #[test]
    fn should_warn_when_script_src_excludes_self() {
        // given
        let policy = "script-src https://cdn.example.test/x.js 'nonce-abc'; connect-src 'self'";
        let hostless = "script-src 'nonce-abc' 'sha256-xyz' data:";

        // when
        let (with_host, without_host) = (detect(&[policy]), detect(&[hostless]));

        // then
        // A host-source may match this server, so only the host-less list is certain.
        assert_eq!(with_host, None);
        assert_eq!(
            without_host,
            Some(CspWarning {
                script: Some("script-src"),
                connect: None
            })
        );
    }

    #[test]
    fn should_warn_when_default_src_none_and_no_connect_src() {
        // given
        let policy = "default-src 'none'; img-src 'self'";

        // when
        let warning = detect(&[policy]);

        // then
        assert_eq!(
            warning,
            Some(CspWarning {
                script: Some("default-src"),
                connect: Some("default-src")
            })
        );
    }

    #[test]
    fn should_not_warn_when_self_allowed() {
        // given
        let policies = [
            "default-src 'self'",
            "script-src 'self'; connect-src 'self'",
            "script-src 'SELF' 'nonce-a'; default-src 'none'; connect-src 'self'",
            "img-src 'none'",
            "",
            "script-src *; connect-src http:",
            "default-src https://cdn.example.test 'self'",
            "default-src 'none'; script-src 'self'; connect-src 'self'",
        ];

        // when
        let warnings = policies.map(|p| detect(&[p]));

        // then
        assert!(warnings.iter().all(Option::is_none), "{warnings:?}");
    }

    #[test]
    fn should_warn_when_only_https_scheme_allowed() {
        // given
        // This server is plain http, so `https:` never matches it.
        let default_https = "default-src https:";
        let script_https = "script-src https: 'unsafe-inline'; connect-src 'self'";

        // when
        let (default, script) = (detect(&[default_https]), detect(&[script_https]));

        // then
        assert_eq!(
            default,
            Some(CspWarning {
                script: Some("default-src"),
                connect: Some("default-src")
            })
        );
        assert_eq!(
            script,
            Some(CspWarning {
                script: Some("script-src"),
                connect: None
            })
        );
    }

    #[test]
    fn should_follow_fallback_order_when_directives_overlap() {
        // given
        // script-src-elem wins over script-src, which wins over default-src.
        let elem_none = "script-src-elem 'none'; script-src 'self'";
        let elem_self = "script-src-elem 'self'; script-src 'none'";
        let script_none = "script-src 'none'; default-src 'self'";
        let connect_none = "connect-src 'none'; default-src 'self'";
        let connect_self = "connect-src 'self'; default-src 'none'";
        // The first of a repeated directive counts.
        let repeated = "script-src 'self'; script-src 'none'";

        // when
        let results = [
            script(elem_none),
            script(elem_self),
            script(script_none),
            connect(connect_none),
            connect(connect_self),
            script(repeated),
        ];

        // then
        assert_eq!(
            results,
            [
                Some("script-src-elem"),
                None,
                Some("script-src"),
                Some("connect-src"),
                None,
                None
            ]
        );
    }

    #[test]
    fn should_block_when_list_is_empty_none_or_strict_dynamic() {
        // given
        let empty = "connect-src";
        let none = "connect-src 'none'";
        let strict = "script-src 'self' 'strict-dynamic' 'nonce-a'";
        let none_and_self = "connect-src 'none' 'self'";
        let schemes = "connect-src data: blob:";

        // when
        let results = [
            connect(empty),
            connect(none),
            script(strict),
            connect(none_and_self),
            connect(schemes),
        ];

        // then
        // 'none' next to another source is ignored by browsers.
        assert_eq!(
            results,
            [
                Some("connect-src"),
                Some("connect-src"),
                Some("script-src"),
                None,
                Some("connect-src")
            ]
        );
    }

    #[test]
    fn should_warn_when_any_of_several_policies_blocks() {
        // given
        let metas = [
            "script-src 'self'; connect-src 'self'",
            "connect-src 'none'",
        ];
        let comma = ["script-src 'self', connect-src 'none'"];

        // when
        let (many, one) = (detect(&metas), detect(&comma));

        // then
        let expected = Some(CspWarning {
            script: None,
            connect: Some("connect-src"),
        });
        assert_eq!(many, expected);
        assert_eq!(one, expected);
    }

    #[test]
    fn should_not_warn_when_no_policy() {
        // given
        let none: [&str; 0] = [];

        // when
        let warning = detect(&none);

        // then
        assert_eq!(warning, None);
    }

    #[test]
    fn should_name_directives_when_displayed() {
        // given
        let warning = CspWarning {
            script: Some("script-src"),
            connect: Some("default-src"),
        };

        // when
        let text = warning.to_string();

        // then
        assert!(
            text.contains("script-src") && text.contains("default-src"),
            "{text}"
        );
        assert!(!text.contains('\n'));
    }
}
