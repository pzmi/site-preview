//! The divergence report (foundation.md section 6.3, step 6): model, seed, trace id, step,
//! action, expected state, actual state and a minimal line diff between the two.
//!
//! The format is plain text, stable enough for a test to look for a line, and each field is
//! on its own line so a CI log can be searched:
//!
//! ```text
//! conformance divergence: the observation differs from the model
//!   model:    formal/lifecycle.qnt
//!   seed:     11
//!   trace:    itf-decoder-sample.itf.json
//!   step:     4
//!   action:   stopServer
//!   params:   (none)
//!   expected:
//!     Obs {
//!         listeners: 0,
//!     }
//!   actual:
//!     Obs {
//!         listeners: 1,
//!     }
//!   diff (- expected, + actual):
//!       Obs {
//!     -     listeners: 0,
//!     +     listeners: 1,
//!       }
//! ```

use std::fmt;

use crate::replay::{Cause, Divergence};

/// Lines of unchanged context kept around each change in the diff.
const CONTEXT: usize = 2;

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let unknown =
            |field: &Option<String>| field.clone().unwrap_or_else(|| "(not recorded)".into());
        match &self.cause {
            Cause::Mismatch => writeln!(
                f,
                "conformance divergence: the observation differs from the model"
            )?,
            Cause::Driver(err) => writeln!(f, "conformance divergence: {err}")?,
            Cause::Projection(err) => {
                writeln!(
                    f,
                    "conformance divergence: cannot project the model state: {err}"
                )?;
            }
            Cause::Trace(why) => writeln!(f, "conformance divergence: unusable trace: {why}")?,
        }
        writeln!(f, "  model:    {}", unknown(&self.model))?;
        writeln!(f, "  seed:     {}", unknown(&self.seed))?;
        writeln!(f, "  trace:    {}", self.trace_id)?;
        writeln!(f, "  step:     {}", self.step)?;
        writeln!(f, "  action:   {}", self.action)?;
        let params = if self.params.is_empty() {
            "(none)"
        } else {
            &self.params
        };
        writeln!(f, "  params:   {params}")?;
        for (label, text) in [("expected", &self.expected), ("actual", &self.actual)] {
            match text {
                Some(text) => {
                    writeln!(f, "  {label}:")?;
                    for line in text.lines() {
                        writeln!(f, "    {line}")?;
                    }
                }
                None => writeln!(f, "  {label}: (not available)")?,
            }
        }
        if let (Some(expected), Some(actual)) = (&self.expected, &self.actual)
            && expected != actual
        {
            writeln!(f, "  diff (- expected, + actual):")?;
            for line in line_diff(expected, actual) {
                writeln!(f, "    {line}")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for Divergence {}

/// One line of a diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edit<'a> {
    Same(&'a str),
    Removed(&'a str),
    Added(&'a str),
}

/// A line diff of `expected` against `actual` (longest common subsequence), keeping
/// [`CONTEXT`] unchanged lines around each change and eliding the rest as `...`. Each line is
/// prefixed `- ` (only in `expected`), `+ ` (only in `actual`) or two spaces.
pub fn line_diff(expected: &str, actual: &str) -> Vec<String> {
    let edits = edits(
        &expected.lines().collect::<Vec<_>>(),
        &actual.lines().collect::<Vec<_>>(),
    );
    let near_change = |i: usize| {
        let lo = i.saturating_sub(CONTEXT);
        let hi = (i + CONTEXT + 1).min(edits.len());
        edits[lo..hi].iter().any(|e| !matches!(e, Edit::Same(_)))
    };
    let mut out = Vec::new();
    let mut elided = false;
    for (i, edit) in edits.iter().enumerate() {
        if !near_change(i) {
            if !elided {
                out.push("  ...".to_owned());
                elided = true;
            }
            continue;
        }
        elided = false;
        out.push(match edit {
            Edit::Same(line) => format!("  {line}"),
            Edit::Removed(line) => format!("- {line}"),
            Edit::Added(line) => format!("+ {line}"),
        });
    }
    out
}

fn edits<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Edit<'a>> {
    // lcs[i][j]: the longest common subsequence of old[i..] and new[j..].
    let mut lcs = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lcs[i][j] = if old[i] == new[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::with_capacity(old.len() + new.len());
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            out.push(Edit::Same(old[i]));
            i += 1;
            j += 1;
        } else if j < new.len() && (i == old.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
            out.push(Edit::Added(new[j]));
            j += 1;
        } else {
            out.push(Edit::Removed(old[i]));
            i += 1;
        }
    }
    // Show each change's removals before its additions.
    for k in 1..out.len() {
        let mut m = k;
        while m > 0 && matches!(out[m], Edit::Removed(_)) && matches!(out[m - 1], Edit::Added(_)) {
            out.swap(m, m - 1);
            m -= 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::DriverError;

    fn divergence(cause: Cause, expected: Option<&str>, actual: Option<&str>) -> Divergence {
        Divergence {
            model: Some("formal/toy.qnt".into()),
            seed: None,
            trace_id: "toy.itf.json".into(),
            step: 3,
            action: "tick".into(),
            params: "n = 2".into(),
            expected: expected.map(str::to_owned),
            actual: actual.map(str::to_owned),
            cause,
        }
    }

    #[test]
    fn should_keep_only_changed_lines_and_context_when_diffing() {
        // given
        let expected = "a\nb\nc\nd\ne\nf\ng\nh";
        let actual = "a\nb\nc\nd\nE\nf\ng\nh";

        // when
        let diff = line_diff(expected, actual);

        // then
        assert_eq!(
            diff,
            ["  ...", "  c", "  d", "- e", "+ E", "  f", "  g", "  ..."]
        );
    }

    #[test]
    fn should_mark_insertions_and_deletions_when_line_counts_differ() {
        // given
        let expected = "x\ny";
        let actual = "x\nz\ny\nw";

        // when
        let diff = line_diff(expected, actual);

        // then
        assert_eq!(diff, ["  x", "+ z", "  y", "+ w"]);
    }

    #[test]
    fn should_name_every_field_when_formatting_a_mismatch() {
        // given
        let d = divergence(Cause::Mismatch, Some("n: 1"), Some("n: 2"));

        // when
        let report = d.to_string();

        // then
        assert_eq!(
            report,
            "conformance divergence: the observation differs from the model\n\
             \x20 model:    formal/toy.qnt\n\
             \x20 seed:     (not recorded)\n\
             \x20 trace:    toy.itf.json\n\
             \x20 step:     3\n\
             \x20 action:   tick\n\
             \x20 params:   n = 2\n\
             \x20 expected:\n\
             \x20   n: 1\n\
             \x20 actual:\n\
             \x20   n: 2\n\
             \x20 diff (- expected, + actual):\n\
             \x20   - n: 1\n\
             \x20   + n: 2\n"
        );
    }

    #[test]
    fn should_state_the_refusal_when_the_driver_failed() {
        // given
        let d = divergence(
            Cause::Driver(DriverError::Refused("bind failed".into())),
            Some("n: 1"),
            Some("n: 1"),
        );

        // when
        let report = d.to_string();

        // then
        assert!(
            report.starts_with("conformance divergence: the implementation refused: bind failed\n")
        );
        assert!(!report.contains("diff"), "{report}");
    }

    #[test]
    fn should_say_not_available_when_a_state_is_missing() {
        // given
        let d = divergence(Cause::Trace("no states".into()), None, None);

        // when
        let report = d.to_string();

        // then
        assert!(report.contains("  expected: (not available)\n"), "{report}");
        assert!(report.contains("unusable trace: no states"), "{report}");
    }
}
