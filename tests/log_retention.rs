#![cfg(target_os = "linux")]

mod common;

use common::TestDir;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_logcut")
}

#[test]
fn log_age_boundary_keeps_newer_logs_and_removes_older_logs() {
    let root = TestDir::new("logcut-retention", "age-boundary");
    let logs = root.join("logs");
    fs::create_dir_all(&logs).unwrap();
    fs::set_permissions(&logs, fs::Permissions::from_mode(0o700)).unwrap();

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
        .env("LOGCUT_LOG_MAX_AGE_DAYS", "1")
        .env("LOGCUT_LOG_MAX_FILES", "10")
        .args(["sh", "-c", "printf failure; exit 1"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(newer.exists(), "log newer than one day was removed");
    assert!(!older.exists(), "log older than one day was retained");
}
