#![cfg(target_os = "linux")]

mod common;

use common::TestDir;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Output};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_logcut")
}

fn temp_dir(name: &str) -> TestDir {
    TestDir::new("logcut-test", name)
}

fn run_with_umask(logs: &Path, destination: &Path, mask: libc::mode_t) -> Output {
    let mut command = Command::new(binary());
    command.env("LOGCUT_LOG_DIRECTORY", logs).args([
        "sh",
        "-c",
        "printf data >\"$1\"",
        "_",
        destination.to_str().unwrap(),
    ]);

    unsafe {
        command.pre_exec(move || {
            libc::umask(mask);
            Ok(())
        });
    }

    command.output().unwrap()
}

fn assert_mode(path: &Path, expected: u32) {
    let actual = fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(actual, expected, "unexpected mode for {}", path.display());
}

#[test]
fn target_command_uses_callers_umask() {
    let root = temp_dir("umask-suppressed");
    let logs = root.join("logs");
    let destination = root.join("created.txt");

    let output = run_with_umask(&logs, &destination, 0o027);

    assert!(output.status.success());
    assert_mode(&destination, 0o640);
    assert_mode(&logs, 0o700);
}

#[test]
fn logging_failure_does_not_run_target_command() {
    let root = temp_dir("umask-fail-closed");
    let logs = root.join("logs");
    let destination = root.join("created.txt");
    fs::write(&logs, b"not a directory").unwrap();

    let output = run_with_umask(&logs, &destination, 0o027);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(1));
    assert!(!destination.exists());
    assert!(stderr.contains("secure logging is unavailable"));
    assert!(stderr.contains("command was not executed"));
    assert!(stderr.contains("Run the command without logcut"));
}
