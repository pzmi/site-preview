//! Seeded random traces (foundation.md section 6.4), in `formal/traces/generated/`, which Git
//! ignores.
//!
//! Under `cargo xtask conformance` the `formal-generate` stage writes them once and records
//! the seed, the Quint version and each family's count and length in
//! `formal/traces/generated/manifest.json`; it sets [`MANIFEST_ENV`] for the replay, and
//! [`Family::traces`] reads its family from that manifest, refusing one whose parameters
//! differ from the test's (ADR 0060). Run directly (`cargo nextest run -p
//! preview-conformance`), a test generates its own family with the locked Quint
//! (`formal/package-lock.json`) as `npx --prefix formal --no -- quint run --backend
//! typescript --mbt`, never fetched and never the downloaded Rust evaluator (ADR 0052), and
//! removes the manifest, which no longer describes the directory. The seed comes from
//! `QUINT_SEED` when set, so a failing run repeats exactly. A missing Node or Quint is an
//! error naming the install command, never a skip.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Set by `cargo xtask conformance` to the generated traces' manifest, relative to the
/// repository root: the replay then reads its traces from there instead of generating them.
pub const MANIFEST_ENV: &str = "CONFORMANCE_GENERATED_MANIFEST";

/// The repository's `formal/` directory.
pub fn formal_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../formal")
}

/// The seed for this run: `QUINT_SEED` when set and not empty, else one from the clock.
pub fn seed() -> String {
    std::env::var("QUINT_SEED")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            format!("{:#x}", nanos & u128::from(u64::MAX))
        })
}

/// One family of generated traces.
#[derive(Clone, Debug)]
pub struct Family<'a> {
    /// The model file in `formal/`, such as `lifecycle.qnt`.
    pub file: &'a str,
    /// Its main module.
    pub main: &'a str,
    /// The step relation to sample.
    pub step: &'a str,
    /// The invariant every generated state must satisfy (a definition name).
    pub invariant: &'a str,
    /// How many traces.
    pub traces: u32,
    /// Steps per trace at most.
    pub max_steps: u32,
    /// The file-name prefix in `formal/traces/generated/`; earlier files with it are removed.
    pub prefix: &'a str,
}

impl Family<'_> {
    /// A one-line record of the run: family, count, length and seed, so the test output (and
    /// a failure report) says how to repeat it.
    pub fn describe(&self, seed: &str) -> String {
        format!(
            "{} --step {}: {} traces of at most {} steps, seed {seed} \
             (QUINT_SEED={seed} repeats it)",
            self.file, self.step, self.traces, self.max_steps
        )
    }

    /// The traces to replay and their seed: the manifest's when [`MANIFEST_ENV`] is set,
    /// otherwise fresh ones generated with [`seed`].
    ///
    /// # Errors
    ///
    /// A message when the manifest cannot be read, lacks this family or records other
    /// parameters, or when generation fails.
    pub fn traces(&self) -> Result<(String, Vec<PathBuf>), String> {
        match std::env::var(MANIFEST_ENV) {
            Ok(path) if !path.is_empty() => {
                let manifest = formal_dir().join("..").join(&path);
                let text = std::fs::read_to_string(&manifest).map_err(|e| {
                    format!(
                        "cannot read {path} ({MANIFEST_ENV}): {e}; run 'cargo xtask conformance'"
                    )
                })?;
                let dir = manifest.parent().unwrap_or(Path::new("."));
                self.from_manifest(&text, dir)
            }
            _ => {
                let manifest = formal_dir().join("traces/generated/manifest.json");
                match std::fs::remove_file(&manifest) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        return Err(format!("cannot remove {}: {e}", manifest.display()));
                    }
                    _ => {}
                }
                let seed = seed();
                let files = self.generate(&seed)?;
                Ok((seed, files))
            }
        }
    }

    /// This family's seed and trace paths (under `dir`) from a generated manifest's `text`.
    /// The entry is the one with this model file and step relation; its main module,
    /// invariant, count, length and status must match, and it must list `traces` files.
    ///
    /// # Errors
    ///
    /// A message naming what is missing or differs.
    pub fn from_manifest(&self, text: &str, dir: &Path) -> Result<(String, Vec<PathBuf>), String> {
        let json: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("the generated manifest: {e}"))?;
        let seed = json["seed"]
            .as_str()
            .ok_or("the generated manifest has no seed")?
            .to_owned();
        let entry = json["families"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|f| f["model"] == self.file && f["step"] == self.step)
            .ok_or_else(|| {
                format!(
                    "the generated manifest has no family for {} --step {}",
                    self.file, self.step
                )
            })?;
        let expected = [
            ("main", serde_json::Value::from(self.main)),
            ("invariant", serde_json::Value::from(self.invariant)),
            ("traces", serde_json::Value::from(self.traces)),
            ("max_steps", serde_json::Value::from(self.max_steps)),
            ("status", serde_json::Value::from("ok")),
        ];
        for (key, want) in expected {
            if entry[key] != want {
                return Err(format!(
                    "the generated manifest records {key} {} for {} --step {}, the test wants {want}",
                    entry[key], self.file, self.step
                ));
            }
        }
        let files: Vec<PathBuf> = entry["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .map(|f| dir.join(f))
            .collect();
        if files.len() != self.traces as usize {
            return Err(format!(
                "the generated manifest lists {} traces for {} --step {}, not {}",
                files.len(),
                self.file,
                self.step,
                self.traces
            ));
        }
        Ok((seed, files))
    }

    /// Generates the traces with `seed` and returns their paths, sorted.
    ///
    /// # Errors
    ///
    /// A message when Quint cannot run, rejects the model, finds an invariant violated or
    /// writes fewer traces than asked for.
    pub fn generate(&self, seed: &str) -> Result<Vec<PathBuf>, String> {
        let formal = formal_dir();
        let out = formal.join("traces").join("generated");
        std::fs::create_dir_all(&out)
            .map_err(|e| format!("cannot create formal/traces/generated: {e}"))?;
        for stale in self.files_in(&out)? {
            std::fs::remove_file(&stale)
                .map_err(|e| format!("cannot remove {}: {e}", stale.display()))?;
        }
        let pattern = out.join(format!("{}{{seq}}.itf.json", self.prefix));
        let output = Command::new("npx")
            .arg("--prefix")
            .arg(&formal)
            .args(["--no", "--", "quint", "run", self.file])
            .args(["--backend", "typescript", "--mbt", "--main", self.main])
            .args(["--step", self.step, "--invariant", self.invariant])
            .arg("--max-samples")
            .arg(self.traces.to_string())
            .arg("--n-traces")
            .arg(self.traces.to_string())
            .arg("--max-steps")
            .arg(self.max_steps.to_string())
            .args(["--seed", seed, "--out-itf"])
            .arg(&pattern)
            .current_dir(&formal)
            .output()
            .map_err(|e| {
                format!(
                    "cannot run npx ({e}): install Node, then run 'npm ci --prefix formal' \
                     (Quint is locked by formal/package-lock.json)"
                )
            })?;
        if !output.status.success() {
            return Err(format!(
                "quint run failed for {} (if Quint is missing, run 'npm ci --prefix formal'):\n\
                 {}{}",
                self.describe(seed),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let files = self.files_in(&out)?;
        if files.len() != self.traces as usize {
            return Err(format!(
                "quint wrote {} traces, not {}, for {}",
                files.len(),
                self.traces,
                self.describe(seed)
            ));
        }
        Ok(files)
    }

    /// The files in `dir` with this family's prefix, sorted.
    fn files_in(&self, dir: &Path) -> Result<Vec<PathBuf>, String> {
        let entries =
            std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(self.prefix) && n.ends_with(".itf.json"))
            })
            .collect();
        files.sort();
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAMILY: Family<'static> = Family {
        file: "reload.qnt",
        main: "reload",
        step: "step",
        invariant: "allInvariants",
        traces: 2,
        max_steps: 25,
        prefix: "reload-",
    };

    fn manifest(traces: u32, files: &str) -> String {
        format!(
            "{{\"seed\": \"0x2a\", \"quint\": \"0.33.0\", \"families\": [\
             {{\"family\": \"lifecycle\", \"model\": \"lifecycle.qnt\", \"step\": \"serverStep\"}},\
             {{\"family\": \"reload\", \"model\": \"reload.qnt\", \"main\": \"reload\", \
             \"step\": \"step\", \"invariant\": \"allInvariants\", \"traces\": {traces}, \
             \"max_steps\": 25, \"status\": \"ok\", \"files\": [{files}]}}]}}"
        )
    }

    #[test]
    fn should_return_the_family_files_and_seed_when_the_manifest_matches() {
        // given
        let text = manifest(2, "\"reload-0.itf.json\", \"reload-1.itf.json\"");

        // when
        let result = FAMILY.from_manifest(&text, Path::new("gen"));

        // then
        assert_eq!(
            result,
            Ok((
                "0x2a".to_owned(),
                vec![
                    PathBuf::from("gen/reload-0.itf.json"),
                    PathBuf::from("gen/reload-1.itf.json")
                ]
            ))
        );
    }

    #[test]
    fn should_refuse_the_manifest_when_it_records_another_budget() {
        // given
        let text = manifest(3, "\"reload-0.itf.json\"");

        // when
        let result = FAMILY.from_manifest(&text, Path::new("gen"));

        // then
        let message = result.expect_err("another count");
        assert!(message.contains("records traces 3"), "{message}");
    }

    #[test]
    fn should_refuse_the_manifest_when_files_are_missing_or_the_family_is_absent() {
        // given
        let short = manifest(2, "\"reload-0.itf.json\"");
        let other = Family {
            step: "other",
            ..FAMILY
        };

        // when
        let missing = FAMILY.from_manifest(&short, Path::new("gen"));
        let absent = other.from_manifest(&short, Path::new("gen"));

        // then
        assert!(missing.expect_err("one file").contains("lists 1 traces"));
        assert!(
            absent
                .expect_err("no family")
                .contains("no family for reload.qnt --step other")
        );
    }
}
