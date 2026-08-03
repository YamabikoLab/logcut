#![cfg(target_os = "linux")]

mod common;

use common::{TestDir, LOG_DIRECTORY_MARKER};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_logcut")
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn empty_existing_directory_is_initialized_without_changing_permissions() {
    let root = TestDir::new("logcut-issue-49", "empty-directory");
    let logs = root.join("custom-logs");
    fs::create_dir_all(&logs).unwrap();
    fs::set_permissions(&logs, fs::Permissions::from_mode(0o700)).unwrap();

    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .args(["sh", "-c", "printf failure; exit 7"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(7));
    assert!(logs.join(LOG_DIRECTORY_MARKER).is_file());
    assert_eq!(
        fs::metadata(&logs).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[test]
fn existing_directory_permissions_are_not_changed() {
    let root = TestDir::new("logcut-issue-49", "permissions");
    let logs = root.join("custom-logs");
    fs::create_dir_all(&logs).unwrap();
    fs::set_permissions(&logs, fs::Permissions::from_mode(0o755)).unwrap();
    let existing = logs.join("keep.txt");
    fs::write(&existing, "keep").unwrap();

    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .args(["sh", "-c", "printf direct-output; exit 23"])
        .output()
        .unwrap();
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(23));
    assert!(text.contains("secure logging is unavailable"));
    assert!(text.contains("direct-output"));
    assert_eq!(
        fs::metadata(&logs).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(fs::read_to_string(existing).unwrap(), "keep");
    assert!(!logs.join(LOG_DIRECTORY_MARKER).exists());
}

#[test]
fn unmarked_nonempty_directory_is_not_pruned() {
    let root = TestDir::new("logcut-issue-49", "unmarked");
    let logs = root.join("custom-logs");
    fs::create_dir_all(&logs).unwrap();
    fs::set_permissions(&logs, fs::Permissions::from_mode(0o700)).unwrap();
    let existing = logs.join("command.user-owned.log");
    fs::write(&existing, "do not remove").unwrap();

    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .env("LOGCUT_LOG_MAX_FILES", "1")
        .args(["sh", "-c", "printf direct-output; exit 29"])
        .output()
        .unwrap();
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(29));
    assert!(text.contains("secure logging is unavailable"));
    assert_eq!(fs::read_to_string(existing).unwrap(), "do not remove");
    assert_eq!(fs::read_dir(&logs).unwrap().count(), 1);
}

#[test]
fn invalid_marker_is_rejected_without_replacing_it() {
    let root = TestDir::new("logcut-issue-49", "invalid-marker");
    let logs = root.join("custom-logs");
    fs::create_dir_all(&logs).unwrap();
    fs::set_permissions(&logs, fs::Permissions::from_mode(0o700)).unwrap();
    let marker = logs.join(LOG_DIRECTORY_MARKER);
    fs::write(&marker, "not logcut\n").unwrap();
    fs::set_permissions(&marker, fs::Permissions::from_mode(0o600)).unwrap();

    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .args(["sh", "-c", "printf direct-output; exit 31"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(31));
    assert!(combined(&output).contains("secure logging is unavailable"));
    assert_eq!(fs::read_to_string(marker).unwrap(), "not logcut\n");
}
