//! Running external commands as gate stages.

use std::process::Command;

/// One command to run as part of a gate.
#[derive(Debug, Clone)]
pub struct Stage {
    /// Short name printed in the progress line.
    pub name: &'static str,
    /// Program to execute.
    pub program: String,
    /// Arguments passed to the program.
    pub args: Vec<String>,
}

impl Stage {
    /// Builds a stage from string slices.
    pub fn new(name: &'static str, program: &str, args: &[&str]) -> Self {
        Self {
            name,
            program: program.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        }
    }
}

/// Runs stages in order, streaming their output, and stops at the first failure.
///
/// Returns 0 when all pass, otherwise the failing stage's exit code (1 when the process was
/// killed by a signal or could not be spawned).
pub fn run_stages(stages: &[Stage]) -> i32 {
    for stage in stages {
        eprintln!(
            "xtask: stage {}: {} {}",
            stage.name,
            stage.program,
            stage.args.join(" ")
        );
        let code = match Command::new(&stage.program).args(&stage.args).status() {
            Ok(status) => status.code().unwrap_or(1),
            Err(err) => {
                eprintln!("xtask: cannot run {}: {err}", stage.program);
                1
            }
        };
        if code != 0 {
            eprintln!("xtask: stage {} FAILED with exit code {code}", stage.name);
            return code;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(name: &'static str, script: &str) -> Stage {
        Stage::new(name, "sh", &["-c", script])
    }

    #[test]
    fn should_return_failing_exit_code_when_a_stage_fails() {
        // given
        let stages = [sh("ok", "exit 0"), sh("bad", "exit 7")];

        // when
        let code = run_stages(&stages);

        // then
        assert_eq!(code, 7);
    }

    #[test]
    fn should_skip_later_stages_when_an_earlier_stage_fails() {
        // given
        let dir = tempfile::TempDir::new().unwrap();
        let marker = dir.path().join("ran");
        let marker_arg = marker.to_str().unwrap();
        let stages = [
            sh("bad", "exit 4"),
            Stage::new("later", "sh", &["-c", "touch \"$1\"", "sh", marker_arg]),
        ];

        // when
        let code = run_stages(&stages);

        // then
        assert_eq!(code, 4);
        assert!(!marker.exists(), "later stage must not run");
    }

    #[test]
    fn should_return_zero_when_all_stages_pass() {
        // given
        let stages = [sh("a", "exit 0"), sh("b", "true")];

        // when
        let code = run_stages(&stages);

        // then
        assert_eq!(code, 0);
    }

    #[test]
    fn should_fail_when_program_does_not_exist() {
        // given
        let stages = [Stage::new("missing", "xtask-no-such-program", &[])];

        // when
        let code = run_stages(&stages);

        // then
        assert_eq!(code, 1);
    }
}
