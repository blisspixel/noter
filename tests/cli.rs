//! Black-box command-line behavior tests for the Noter executable.

use std::ffi::{OsStr, OsString};
use std::process::{Command, Output};

fn run_noter<I, S>(arguments: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_noter"))
        .args(arguments)
        .output()
        .expect("the Noter executable should run")
}

#[test]
fn invalid_arguments_exit_unsuccessfully_with_actionable_guidance() {
    let output = run_noter(["--theme", "blue"]);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
    assert!(stderr.contains("unknown theme `blue`; expected system, light, dark, green, or amber"));
    assert!(stderr.contains("Usage:"));
}

#[test]
fn help_and_version_exit_successfully_without_starting_the_gui() {
    let help = run_noter(["--help"]);
    assert!(help.status.success());
    assert!(help.stderr.is_empty());
    let help_text = String::from_utf8(help.stdout).expect("help should be valid UTF-8");
    assert!(help_text.contains("noter [OPTIONS] [--] [FILE]"));
    assert!(help_text.contains("--theme system|light|dark|green|amber"));

    let version = run_noter(["--version"]);
    assert!(version.status.success());
    assert!(version.stderr.is_empty());
    assert_eq!(
        String::from_utf8(version.stdout).expect("version should be valid UTF-8"),
        format!("noter {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[cfg(unix)]
fn non_unicode_option() -> OsString {
    use std::os::unix::ffi::OsStringExt;

    OsString::from_vec(vec![b'-', 0xff])
}

#[cfg(windows)]
fn non_unicode_option() -> OsString {
    use std::os::windows::ffi::OsStringExt;

    OsString::from_wide(&[u16::from(b'-'), 0xd800])
}

#[test]
fn non_unicode_option_exits_with_a_controlled_error_instead_of_panicking() {
    let output = run_noter([non_unicode_option()]);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
    assert!(stderr.contains("option names must be valid Unicode"));
    assert!(!stderr.contains("panicked"));
}
