//! Seeded random traces generated at test time (foundation.md section 6.4): the locked Quint
//! (`formal/package-lock.json`) run as `npx --prefix formal --no -- quint run --backend
//! typescript --mbt`, never fetched and never the downloaded Rust evaluator (ADR 0052).
//! The traces land in `formal/traces/generated/`, which Git ignores; the seed comes from
//! `QUINT_SEED` when set, so a failing run repeats exactly. A missing Node or Quint is an
//! error naming the install command, never a skip.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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
