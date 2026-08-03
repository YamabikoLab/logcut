#![cfg(target_os = "linux")]

mod common;

use common::{prepare_log_directory, TestDir};
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

fn log_files(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("command.") && name.ends_with(".log"))
        })
        .collect()
}

#[test]
fn failure_log_is_retained_by_default() {
    let root = TestDir::new("logcut-issue-36", "default-retain");
    let logs = root.join("logs");
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .args(["sh", "-c", "printf retained; exit 17"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(17));
    assert!(combined(&output).contains("Full log:"));
    assert_eq!(log_files(&logs).len(), 1);
}

#[test]
fn environment_discards_failure_log_after_summary() {
    let root = TestDir::new("logcut-issue-36", "environment-discard");
    let logs = root.join("logs");
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .env("LOGCUT_RETAIN_FAILED_LOG", "0")
        .args(["sh", "-c", "printf discarded; exit 23"])
        .output()
        .unwrap();
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(23));
    assert!(text.contains("discarded"));
    assert!(text.contains("Full log discarded."));
    assert!(!text.contains("Full log:"));
    assert!(log_files(&logs).is_empty());
}

#[test]
fn cli_option_overrides_environment_retention() {
    let root = TestDir::new("logcut-issue-36", "cli-overrides-environment");
    let logs = root.join("logs");
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .env("LOGCUT_RETAIN_FAILED_LOG", "1")
        .arg("--no-retain-log")
        .args(["sh", "-c", "printf cli-discarded; exit 29"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(29));
    assert!(combined(&output).contains("Full log discarded."));
    assert!(log_files(&logs).is_empty());
}

#[test]
fn discard_is_attempted_when_log_reading_fails() {
    let root = TestDir::new("logcut-issue-36", "read-failure");
    let logs = root.join("logs");
    let script = "rm -f \"$LOGCUT_LOG_DIRECTORY\"/command.*.log; exit 31";
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .arg("--no-retain-log")
        .args(["sh", "-c", script])
        .output()
        .unwrap();
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(31));
    assert!(text.contains("failure summary could not be generated"));
    assert!(text.contains("Full log discarded."));
    assert!(log_files(&logs).is_empty());
}

#[test]
fn discard_failure_after_summary_is_reported() {
    let root = TestDir::new("logcut-issue-36", "discard-failure");
    let logs = root.join("logs");
    let script = "chmod 500 \"$LOGCUT_LOG_DIRECTORY\"; printf sensitive; exit 37";
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .arg("--no-retain-log")
        .args(["sh", "-c", script])
        .output()
        .unwrap();
    let text = combined(&output);

    fs::set_permissions(&logs, fs::Permissions::from_mode(0o700)).unwrap();

    assert_eq!(output.status.code(), Some(37));
    assert!(text.contains("failed to discard full log"));
    assert!(text.contains("Full log may remain:"));
    assert!(!text.contains("Full log discarded."));
    assert_eq!(log_files(&logs).len(), 1);
}

#[test]
fn discard_failure_after_log_read_failure_is_reported() {
    let root = TestDir::new("logcut-issue-36", "read-and-discard-failure");
    let logs = root.join("logs");
    let script = "chmod 000 \"$LOGCUT_LOG_DIRECTORY\"/command.*.log; chmod 500 \"$LOGCUT_LOG_DIRECTORY\"; exit 41";
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .arg("--no-retain-log")
        .args(["sh", "-c", script])
        .output()
        .unwrap();
    let text = combined(&output);

    fs::set_permissions(&logs, fs::Permissions::from_mode(0o700)).unwrap();
    for path in log_files(&logs) {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    assert_eq!(output.status.code(), Some(41));
    assert!(text.contains("failure summary could not be generated"));
    assert!(text.contains("failed to discard full log"));
    assert!(text.contains("Full log may remain:"));
    assert!(!text.contains("Full log discarded."));
    assert_eq!(log_files(&logs).len(), 1);
}

#[test]
fn default_log_age_is_one_day() {
    let root = TestDir::new("logcut-issue-36", "default-age");
    let logs = root.join("logs");
    prepare_log_directory(&logs);

    let newer = logs.join("command.newer.log");
    let older = logs.join("command.older.log");
    fs::write(&newer, "newer").unwrap();
    fs::write(&older, "older").unwrap();
    assert!(Command::new("touch")
        .args(["-d", "23 hours ago"])
        .arg(&newer)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("touch")
        .args(["-d", "25 hours ago"])
        .arg(&older)
        .status()
        .unwrap()
        .success());

    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .args(["sh", "-c", "printf failure; exit 1"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(newer.exists());
    assert!(!older.exists());
}

#[test]
fn explicit_log_age_override_is_preserved() {
    let root = TestDir::new("logcut-issue-36", "age-override");
    let logs = root.join("logs");
    prepare_log_directory(&logs);

    let retained = logs.join("command.retained.log");
    fs::write(&retained, "retained").unwrap();
    assert!(Command::new("touch")
        .args(["-d", "25 hours ago"])
        .arg(&retained)
        .status()
        .unwrap()
        .success());

    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .env("LOGCUT_LOG_MAX_AGE_DAYS", "2")
        .args(["sh", "-c", "printf failure; exit 1"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(retained.exists());
}
