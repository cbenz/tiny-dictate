//! The command line seam: the binary is run as a process, the way the keyboard shortcut and
//! the integration suite run it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh runtime directory per test, so the tests do not see each other's state.
fn runtime_directory(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("tiny-dictate-cli-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("the runtime directory can be created");
    directory
}

fn tiny_dictate(runtime: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tiny-dictate"))
        .args(arguments)
        .env("XDG_RUNTIME_DIR", runtime)
        .env("HOME", runtime)
        .env_remove("XDG_STATE_HOME")
        .output()
        .expect("the binary runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_owned()
}

/// The live dictation publishes its recorder's pid in the runtime directory, the way the
/// bash tool documents it; `status` reads liveness from there.
fn write_pidfile(runtime: &Path, name: &str, pid: &str) {
    let session = runtime.join("tiny-dictate/session");
    fs::create_dir_all(&session).expect("the session directory can be created");
    fs::write(session.join(name), pid).expect("the pidfile can be written");
}

#[test]
fn status_is_idle_when_no_dictation_runs() {
    let runtime = runtime_directory("status-idle");

    let output = tiny_dictate(&runtime, &["status"]);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "idle");
}

#[test]
fn status_is_working_while_the_recorder_lives() {
    let runtime = runtime_directory("status-working");
    write_pidfile(&runtime, "arecord.pid", &std::process::id().to_string());

    let output = tiny_dictate(&runtime, &["status"]);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "working");
}

#[test]
fn status_is_idle_when_the_recorded_process_is_gone() {
    let runtime = runtime_directory("status-stale");
    // Above the largest pid the kernel hands out, so it cannot be alive.
    write_pidfile(&runtime, "arecord.pid", "999999999");

    let output = tiny_dictate(&runtime, &["status"]);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "idle");
}

#[test]
fn an_unknown_verb_prints_the_usage_on_stderr_and_fails() {
    let runtime = runtime_directory("unknown-verb");

    let output = tiny_dictate(&runtime, &["dictate"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    assert!(stderr(&output).contains("Usage: tiny-dictate <command>"));
}

#[test]
fn help_prints_the_usage_and_succeeds() {
    let runtime = runtime_directory("help");

    let output = tiny_dictate(&runtime, &["--help"]);

    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).contains("Usage: tiny-dictate <command>"));
}
