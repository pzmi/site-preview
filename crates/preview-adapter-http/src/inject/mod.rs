//! HTML injection of the live-reload client (plan.md §6).
//!
//! Whether a response is rewritten at all is the domain's [`preview_domain::injection`]
//! decision. [`rewrite`] parses the page with `lol_html`, inserts a
//! [`ScriptTag`] before the real `</body>` end tag (or at document end), refuses to add a second
//! client script, and returns [`Rewrite::Failed`] rather than ever handing back corrupted HTML.
//! The HTML response ties the decision, the encoding check (ADR 0022), the rewrite and the
//! [`HtmlBudget`] for pages held in memory together (ADR 0046).

mod budget;
mod charset;
mod csp;
pub(crate) mod response;
mod rewrite;

use preview_domain::{InstanceId, Revision, SiteState};

pub use budget::{DEFAULT_HTML_BUDGET_BYTES, HtmlBudget};
pub use csp::{CspWarning, detect as detect_csp};
pub use response::Injection;
pub use rewrite::{Rewrite, RewriteFailure, rewrite, rewrite_with_csp};

/// URL path of the embedded live-reload client script (plan.md §6).
pub const CLIENT_SCRIPT_PATH: &str = "/__site_preview/client.js";

/// The `<script>` element injected into served HTML.
///
/// Renders as `<script defer src="/__site_preview/client.js" data-instance="…"
/// data-revision="…"></script>`: an external, deferred script with no inline JavaScript, its
/// data attributes holding the render-time snapshot of the site state (plan.md §6). The values
/// come from a validated [`InstanceId`] and a numeric [`Revision`] and are attribute-escaped
/// regardless (pinned by `should_render_exact_tag_when_built_from_state` and
/// `should_escape_attribute_value_when_value_holds_markup`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptTag(String);

impl ScriptTag {
    /// The tag for `instance` at `revision`.
    #[must_use]
    pub fn new(instance: &InstanceId, revision: Revision) -> Self {
        let mut markup = String::with_capacity(128);
        markup.push_str("<script defer src=\"");
        markup.push_str(CLIENT_SCRIPT_PATH);
        markup.push_str("\" data-instance=\"");
        push_escaped_attribute(&mut markup, instance.as_str());
        markup.push_str("\" data-revision=\"");
        push_escaped_attribute(&mut markup, &revision.get().to_string());
        markup.push_str("\"></script>");
        Self(markup)
    }

    /// The tag for a snapshot of the site state.
    #[must_use]
    pub fn from_state(state: &SiteState) -> Self {
        Self::new(state.instance_id(), state.revision())
    }

    /// The markup to insert.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Appends `value` escaped for a double-quoted HTML attribute: `&`, `<`, `>`, `"` and `'`
/// become character references, so the value can neither end the attribute nor the tag.
fn push_escaped_attribute(out: &mut String, value: &str) {
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_render_exact_tag_when_built_from_state() {
        // given
        let instance = InstanceId::parse("startup-abc").expect("valid instance id");
        let state = SiteState::new(instance, Revision::from(42));

        // when
        let tag = ScriptTag::from_state(&state);

        // then
        assert_eq!(
            tag.as_str(),
            "<script defer src=\"/__site_preview/client.js\" data-instance=\"startup-abc\" \
             data-revision=\"42\"></script>"
        );
    }

    #[test]
    fn should_render_full_width_revision_when_revision_is_max() {
        // given
        let instance = InstanceId::from_parts(u32::MAX, u128::MAX);

        // when
        let tag = ScriptTag::new(&instance, Revision::from(u64::MAX));

        // then
        assert!(
            tag.as_str()
                .contains("data-revision=\"18446744073709551615\"")
        );
        assert!(
            tag.as_str()
                .contains(&format!("data-instance=\"{instance}\""))
        );
    }

    #[test]
    fn should_escape_attribute_value_when_value_holds_markup() {
        // given
        let hostile = "a\"><script>alert('x')</script>&amp;";
        let mut out = String::new();

        // when
        push_escaped_attribute(&mut out, hostile);

        // then
        assert_eq!(
            out,
            "a&quot;&gt;&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;&amp;amp;"
        );
        assert!(!out.contains(['"', '<', '>', '\'']));
    }
}
