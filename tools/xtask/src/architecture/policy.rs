//! The versioned policy file, `architecture.toml` (foundation.md section 3.2, ADR 0033).
//!
//! Parsing is strict: unknown fields fail, and a `schema` other than [`SCHEMA`] fails before
//! any other field is read.

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;
use serde::de::{self, Deserializer};

/// The only schema version this checker reads.
pub const SCHEMA: i64 = 1;

/// The whole policy.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// Schema version; must equal [`SCHEMA`].
    pub schema: i64,
    /// The category of every workspace crate.
    pub workspace: Categories,
    /// Allowed workspace-to-workspace edges.
    pub dependencies: Dependencies,
    /// Allowed third-party crates per workspace crate and dependency kind.
    #[serde(default)]
    pub external: External,
    /// Approved exceptions. No kind of exception is defined yet, so the table must be empty.
    #[serde(default)]
    pub exceptions: Exceptions,
}

/// A crate category. Every workspace crate is in exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    /// Shipped code.
    Production,
    /// Test support, never in a production closure.
    TestOnly,
    /// Repository tooling, never in a production closure.
    ToolOnly,
}

impl Category {
    /// The key under `[workspace]`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::TestOnly => "test_only",
            Self::ToolOnly => "tool_only",
        }
    }
}

/// `[workspace]`: one optional list per category. A list that is present must not be empty.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Categories {
    /// Shipped crates.
    pub production: Option<Vec<String>>,
    /// Test-support crates.
    pub test_only: Option<Vec<String>>,
    /// Tooling crates.
    pub tool_only: Option<Vec<String>>,
}

impl Categories {
    /// Each category with its list, in a fixed order.
    pub fn lists(&self) -> [(Category, Option<&Vec<String>>); 3] {
        [
            (Category::Production, self.production.as_ref()),
            (Category::TestOnly, self.test_only.as_ref()),
            (Category::ToolOnly, self.tool_only.as_ref()),
        ]
    }
}

/// `[dependencies]`: allowed workspace edges by kind.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependencies {
    /// `[dependencies.normal]`: crate to the workspace crates it may depend on. A crate
    /// without an entry may depend on none. Dev-dependencies need no table: they may target
    /// a test_only crate or a crate in this list.
    pub normal: BTreeMap<String, Vec<String>>,
    /// `[dependencies.build]`: crate to the workspace crates it may build-depend on.
    #[serde(default)]
    pub build: BTreeMap<String, Vec<String>>,
}

impl Dependencies {
    /// Each kind's table with its key, in a fixed order.
    pub fn tables(&self) -> [(&'static str, &BTreeMap<String, Vec<String>>); 2] {
        [("normal", &self.normal), ("build", &self.build)]
    }
}

/// Third-party crate name to the features it may enable.
pub type Allowed = BTreeMap<String, Features>;

/// Workspace crate to its [`Allowed`] third-party crates.
pub type ExternalTable = BTreeMap<String, Allowed>;

/// `[external.<kind>]`: workspace crate to the third-party crates it may depend on directly.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct External {
    /// Normal dependencies.
    #[serde(default)]
    pub normal: ExternalTable,
    /// Dev-dependencies. A crate listed under `normal` is also allowed as a dev-dependency.
    #[serde(default)]
    pub dev: ExternalTable,
    /// Build-dependencies.
    #[serde(default)]
    pub build: ExternalTable,
}

impl External {
    /// Each kind's table with its key, in a fixed order.
    pub fn tables(&self) -> [(&'static str, &ExternalTable); 3] {
        [
            ("normal", &self.normal),
            ("dev", &self.dev),
            ("build", &self.build),
        ]
    }
}

/// The features a third-party dependency may enable: `"*"` for any, or a list where
/// `"default"` stands for the dependency's default features.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Features {
    /// Any feature.
    Any,
    /// Only these features and what they imply.
    Only(Vec<String>),
}

impl<'de> Deserialize<'de> for Features {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visit;
        impl<'de> de::Visitor<'de> for Visit {
            type Value = Features;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("\"*\" or a list of feature names")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Features, E> {
                if v == "*" {
                    Ok(Features::Any)
                } else {
                    Err(E::invalid_value(de::Unexpected::Str(v), &self))
                }
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Features, A::Error> {
                let mut out = Vec::new();
                while let Some(name) = seq.next_element::<String>()? {
                    out.push(name);
                }
                Ok(Features::Only(out))
            }
        }
        deserializer.deserialize_any(Visit)
    }
}

/// `[exceptions]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exceptions {}

/// Parses policy text: the schema version first, then every field strictly.
pub fn parse(text: &str) -> Result<Policy, String> {
    let table: toml::Table = text.parse().map_err(|e| format!("not valid TOML: {e}"))?;
    match table.get("schema") {
        Some(toml::Value::Integer(SCHEMA)) => {}
        Some(other) => {
            return Err(format!(
                "unsupported schema version {other}; this checker reads schema = {SCHEMA}"
            ));
        }
        None => return Err(format!("missing `schema = {SCHEMA}`")),
    }
    toml::from_str(text).map_err(|e| e.message().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str =
        "schema = 1\n[workspace]\nproduction = [\"a\"]\n[dependencies.normal]\na = []\n";

    #[test]
    fn should_parse_policy_when_text_is_minimal() {
        // given
        let text = MINIMAL;

        // when
        let policy = parse(text).unwrap();

        // then
        assert_eq!(policy.workspace.production, Some(vec!["a".to_owned()]));
        assert_eq!(policy.workspace.test_only, None);
        assert_eq!(policy.external, External::default());
    }

    #[test]
    fn should_reject_policy_when_schema_version_differs() {
        // given
        let text = MINIMAL.replace("schema = 1", "schema = 2");

        // when
        let err = parse(&text).unwrap_err();

        // then
        assert!(err.contains("unsupported schema version 2"), "{err}");
    }

    #[test]
    fn should_reject_policy_when_schema_is_missing() {
        // given
        let text = MINIMAL.replace("schema = 1\n", "");

        // when
        let err = parse(&text).unwrap_err();

        // then
        assert!(err.contains("missing `schema = 1`"), "{err}");
    }

    #[test]
    fn should_reject_policy_when_a_field_is_unknown() {
        // given
        let text = MINIMAL.replace("production", "prod");

        // when
        let err = parse(&text).unwrap_err();

        // then
        assert!(err.contains("unknown field `prod`"), "{err}");
    }

    #[test]
    fn should_parse_feature_lists_when_external_tables_are_present() {
        // given
        let text = format!(
            "{MINIMAL}[external.normal]\na = {{ tokio = [\"sync\"], log = \"*\" }}\n[external.dev]\na = {{ tempfile = [\"default\"] }}\n"
        );

        // when
        let policy = parse(&text).unwrap();

        // then
        let a = &policy.external.normal["a"];
        assert_eq!(a["tokio"], Features::Only(vec!["sync".to_owned()]));
        assert_eq!(a["log"], Features::Any);
        assert_eq!(
            policy.external.dev["a"]["tempfile"],
            Features::Only(vec!["default".to_owned()])
        );
    }

    #[test]
    fn should_reject_features_when_value_is_another_string() {
        // given
        let text = format!("{MINIMAL}[external.normal]\na = {{ tokio = \"sync\" }}\n");

        // when
        let err = parse(&text).unwrap_err();

        // then
        assert!(err.contains("\"*\" or a list of feature names"), "{err}");
    }

    #[test]
    fn should_reject_exceptions_when_table_is_not_empty() {
        // given
        let text = format!("{MINIMAL}[exceptions]\nanything = 1\n");

        // when
        let err = parse(&text).unwrap_err();

        // then
        assert!(err.contains("unknown field `anything`"), "{err}");
    }
}
