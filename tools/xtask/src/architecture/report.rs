//! Diagnostics of the architecture checker, rendered as `ARCH-<rule>: <message> (at <path>)`.
//!
//! The path is relative to the checked workspace root, never absolute.

use std::fmt;

/// Every rule the checker enforces. The id is what a diagnostic line starts with and what a
/// negative fixture's `expected.txt` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    /// `architecture.toml` is missing, malformed, has an unknown field or another schema.
    PolicySchema,
    /// `cargo metadata` or a crate manifest cannot be read.
    Metadata,
    /// A `[workspace] members` entry has no `Cargo.toml` on disk.
    MissingMember,
    /// A crate under `crates/` or `tools/` is not a workspace member.
    NotAMember,
    /// A workspace crate is in no policy category.
    UnknownCrate,
    /// The policy names a crate that is not in the workspace.
    MissingCrate,
    /// A policy category is present but empty.
    EmptySelector,
    /// A crate is listed in more than one category, or twice in one.
    DuplicateCategory,
    /// A normal workspace edge is not in `[dependencies.normal]`.
    ForbiddenEdge,
    /// A build edge to a workspace crate is not in `[dependencies.build]`.
    UndeclaredBuildDep,
    /// A dev edge targets a crate that is neither test_only nor in the normal allowlist.
    ForbiddenDevEdge,
    /// A production crate reaches a test_only or tool_only crate over normal or build edges.
    ProductionClosure,
    /// The `[dependencies]` allowlist contains a cycle.
    PolicyCycle,
    /// A direct third-party dependency is not in `[external.<kind>]`.
    UnlistedExternal,
    /// A third-party edge enables a feature outside its `[external.<kind>]` list.
    ExternalFeature,
    /// A package comes from git, another registry or a path outside the workspace without
    /// an approved exception.
    ExternalSource,
}

impl Rule {
    /// The rule id, for example `ARCH-unknown-crate`.
    pub fn id(self) -> &'static str {
        match self {
            Self::PolicySchema => "ARCH-policy-schema",
            Self::Metadata => "ARCH-metadata",
            Self::MissingMember => "ARCH-missing-member",
            Self::NotAMember => "ARCH-not-a-member",
            Self::UnknownCrate => "ARCH-unknown-crate",
            Self::MissingCrate => "ARCH-missing-crate",
            Self::EmptySelector => "ARCH-empty-selector",
            Self::DuplicateCategory => "ARCH-duplicate-category",
            Self::ForbiddenEdge => "ARCH-forbidden-edge",
            Self::UndeclaredBuildDep => "ARCH-undeclared-build-dep",
            Self::ForbiddenDevEdge => "ARCH-forbidden-dev-edge",
            Self::ProductionClosure => "ARCH-production-closure",
            Self::PolicyCycle => "ARCH-policy-cycle",
            Self::UnlistedExternal => "ARCH-unlisted-external",
            Self::ExternalFeature => "ARCH-external-feature",
            Self::ExternalSource => "ARCH-external-source",
        }
    }
}

/// One violation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Diagnostic {
    /// The violated rule.
    pub rule: Rule,
    /// What is wrong, naming the crates involved.
    pub message: String,
    /// The offending file, relative to the workspace root.
    pub at: String,
}

impl Diagnostic {
    /// Builds a diagnostic.
    pub fn new(rule: Rule, message: impl Into<String>, at: impl Into<String>) -> Self {
        Self {
            rule,
            message: message.into(),
            at: at.into(),
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} (at {})", self.rule.id(), self.message, self.at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_render_rule_message_and_path_when_displayed() {
        // given
        let diagnostic = Diagnostic::new(
            Rule::UnknownCrate,
            "crate `extra` is in no policy category",
            "crates/extra/Cargo.toml",
        );

        // when
        let line = diagnostic.to_string();

        // then
        assert_eq!(
            line,
            "ARCH-unknown-crate: crate `extra` is in no policy category (at crates/extra/Cargo.toml)"
        );
    }
}
