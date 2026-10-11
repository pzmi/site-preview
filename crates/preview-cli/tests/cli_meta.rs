//! Black-box tests of the meta flags of the `site-preview` binary.

use assert_cmd::Command;
use predicates::prelude::*;

fn site_preview() -> Command {
    Command::cargo_bin("site-preview").expect("the site-preview binary is built")
}

#[test]
fn should_print_version_when_called_with_version_flag() {
    // given
    let mut command = site_preview();
    // when
    let assert = command.arg("--version").assert();
    // then
    assert.success().stdout("site-preview 0.1.0\n");
}

#[test]
fn should_list_serve_when_called_with_help() {
    // given
    let mut command = site_preview();
    // when
    let assert = command.arg("--help").assert();
    // then
    assert.success().stdout(predicate::str::contains("serve"));
}

#[test]
fn should_exit_2_when_given_unknown_flag() {
    // given
    let mut command = site_preview();
    // when
    let assert = command.arg("--bogus").assert();
    // then
    assert.code(2).stderr(predicate::str::contains("Usage"));
}
