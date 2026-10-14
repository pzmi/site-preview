//! The stable codes of every warning the product prints as `warning[<code>]` (ADR 0020).
//!
//! This module is the single registry. A code is a [`WarningCode`], whose field is private to
//! this module, so only the macro below can construct one and every emit site (`warn`,
//! `warning_line`) takes the newtype: a string literal does not compile, and the set cannot
//! grow without being listed here. The `docs` stage of `cargo xtask check` reads this file and
//! fails when a code is missing from the README "Warnings" table or the table lists a code
//! that is not here (ADR 0077). A new code is one line in the macro invocation below. Pinned
//! by `should_list_every_constant_in_all_when_codes_are_declared` and
//! `should_use_lowercase_kebab_case_when_codes_are_declared`.

use std::fmt;

/// A registered warning code; constructible only by the registry below.
///
/// ```compile_fail
/// let _ = preview_domain::warning_code::WarningCode("made-up");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WarningCode(&'static str);

impl WarningCode {
    /// The code as printed in `warning[<code>]`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for WarningCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

macro_rules! warning_codes {
    ($($(#[$meta:meta])* $name:ident = $code:literal;)+) => {
        $($(#[$meta])* pub const $name: WarningCode = WarningCode($code);)+

        /// Every code, in declaration order.
        pub const ALL: &[WarningCode] = &[$($name),+];
    };
}

warning_codes! {
    /// Listening outside loopback with no authentication.
    REMOTE_BINDING = "remote-binding";
    /// `--open` could not start the platform launcher.
    OPEN_FAILED = "open-failed";
    /// An HTML page over the rewrite limit is served unchanged.
    HTML_TOO_LARGE = "html-too-large";
    /// An HTML page that is not UTF-8 is served unchanged.
    HTML_NOT_UTF8 = "html-not-utf8";
    /// The reload script could not be inserted safely.
    HTML_REWRITE_FAILED = "html-rewrite-failed";
    /// The page's meta Content-Security-Policy would block live reload.
    CSP_BLOCKS_LIVE_RELOAD = "csp-blocks-live-reload";
    /// A directory could not be watched.
    WATCH_REGISTRATION_FAILED = "watch-registration-failed";
    /// Nothing can be watched.
    WATCH_UNAVAILABLE = "watch-unavailable";
    /// The native watcher failed while running.
    WATCH_FAILED = "watch-failed";
    /// The revision counter ran out.
    RELOAD_EXHAUSTED = "reload-exhausted";
    /// The site root was deleted, moved or replaced.
    ROOT_REMOVED = "root-removed";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_list_every_constant_in_all_when_codes_are_declared() {
        // given
        let declared = [
            REMOTE_BINDING,
            OPEN_FAILED,
            HTML_TOO_LARGE,
            HTML_NOT_UTF8,
            HTML_REWRITE_FAILED,
            CSP_BLOCKS_LIVE_RELOAD,
            WATCH_REGISTRATION_FAILED,
            WATCH_UNAVAILABLE,
            WATCH_FAILED,
            RELOAD_EXHAUSTED,
            ROOT_REMOVED,
        ];

        // when
        let listed = ALL;

        // then
        assert_eq!(listed, declared);
    }

    #[test]
    fn should_use_lowercase_kebab_case_when_codes_are_declared() {
        // given
        let mut seen = std::collections::BTreeSet::new();

        // when
        let bad: Vec<_> = ALL
            .iter()
            .map(|code| code.as_str())
            .filter(|code| {
                !seen.insert(*code)
                    || code.is_empty()
                    || !code
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            })
            .collect();

        // then
        assert!(bad.is_empty(), "duplicate or malformed codes: {bad:?}");
    }
}
