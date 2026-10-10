//! Running external commands as gate stages.

use std::process::Command;

/// One gate stage: an external command, or a function of this binary.
#[derive(Debug, Clone)]
pub struct Stage {
    /// Short name printed in the progress line; equals its catalog id.
    pub name: &'static str,
    /// What runs.
    pub action: Action,
}

/// How a stage runs.
#[derive(Debug, Clone)]
pub enum Action {
    /// A program and its arguments.
    Command {
        /// Program to execute.
        program: String,
        /// Arguments passed to the program.
        args: Vec<String>,
    },
    /// An xtask function returning an exit code, shown as `display` in the progress line.
    InProcess {
        /// The equivalent command line, for the progress line.
        display: &'static str,
        /// The function.
        run: fn() -> i32,
    },
}

impl Stage {
    /// Builds an external-command stage from string slices.
    pub fn new(name: &'static str, program: &str, args: &[&str]) -> Self {
        Self {
            name,
            action: Action::Command {
                program: program.to_owned(),
                args: args.iter().map(|a| (*a).to_owned()).collect(),
            },
        }
    }

    /// Builds a stage that calls `run` in this process.
    pub fn in_process(name: &'static str, display: &'static str, run: fn() -> i32) -> Self {
        Self {
            name,
            action: Action::InProcess { display, run },
        }
    }

    fn execute(&self) -> i32 {
        match &self.action {
            Action::Command { program, args } => {
                eprintln!("xtask: stage {}: {program} {}", self.name, args.join(" "));
                match Command::new(program).args(args).status() {
                    Ok(status) => status.code().unwrap_or(1),
                    Err(err) => {
                        eprintln!("xtask: cannot run {program}: {err}");
                        1
                    }
                }
            }
            Action::InProcess { display, run } => {
                eprintln!("xtask: stage {}: {display}", self.name);
                run()
            }
        }
    }
}

/// Runs stages in order, streaming their output, and stops at the first failure.
///
/// Returns 0 when all pass, otherwise the failing stage's exit code (1 when the process was
/// killed by a signal or could not be spawned).
pub fn run_stages(stages: &[Stage]) -> i32 {
    for stage in stages {
        let code = stage.execute();
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
    fn should_return_function_exit_code_when_in_process_stage_fails() {
        // given
        let stages = [
            sh("ok", "exit 0"),
            Stage::in_process("inner", "inner", || 5),
        ];

        // when
        let code = run_stages(&stages);

        // then
        assert_eq!(code, 5);
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
