#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

extern "C" {
    fn umask(mask: u32) -> u32;
}

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_logcut")
}

fn temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "logcut-test-{name}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn run_with_umask(logs: &Path, destination: &Path, mask: u32) -> Output {
    let mut command = Command::new(binary());
    command
        .env("LOGCUT_LOG_DIRECTORY", logs)
        .args([
            "sh",
            "-c",
            "printf data >\"$1\"",
            "_",
            destination.to_str().unwrap(),
        ]);

    // SAFETY: This runs in the test child immediately before exec and only sets its umask.
    unsafe {
        command.pre_exec(move || {
            umask(mask);
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
fn direct_fallback_uses_callers_umask() {
    let root = temp_dir("umask-direct");
    let logs = root.join("logs");
    let destination = root.join("created.txt");
    fs::write(&logs, b"not a directory").unwrap();

    let output = run_with_umask(&logs, &destination, 0o027);

    assert!(output.status.success());
    assert_mode(&destination, 0o640);
    assert!(String::from_utf8_lossy(&output.stderr).contains("secure logging is unavailable"));
}
