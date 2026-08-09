#![cfg(target_os = "linux")]

mod common;

use common::{prepare_log_directory, TestDir};
use std::fs;
use std::process::Command;

const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
const TRUNCATION_MARKER: &[u8] = b"[logcut: command output truncated at 10 MiB]";

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_logcut")
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
fn masking_only_truncation_is_reported_and_keeps_the_exit_code() {
    let root = TestDir::new("logcut-issue-52", "masking-only-truncate");
    let logs = root.join("logs");
    prepare_log_directory(&logs);

    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .args([
            "sh",
            "-c",
            "printf '\n[logcut: command output truncated at 10 MiB]\n'; yes AWS_ACCESS_KEY_ID=x | head -n 400000; exit 52",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(52));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr
            .matches("command output exceeded 10 MiB and was truncated")
            .count(),
        1
    );

    let files = log_files(&logs);
    assert_eq!(files.len(), 1);
    let log = fs::read(&files[0]).unwrap();
    assert!(log.len() as u64 <= MAX_LOG_BYTES);
    assert!(!log
        .windows(b"AWS_ACCESS_KEY_ID=x".len())
        .any(|value| value == b"AWS_ACCESS_KEY_ID=x"));
    assert!(log
        .windows(b"[REDACTED]".len())
        .any(|value| value == b"[REDACTED]"));
    assert!(log
        .windows(TRUNCATION_MARKER.len())
        .any(|value| value == TRUNCATION_MARKER));
}

#[test]
fn normal_masking_truncation_is_reported_once_and_keeps_the_exit_code() {
    const LINE_COUNT: u64 = 700_000;
    const RAW_LINE_BYTES: u64 = b"TOKEN=x\n".len() as u64;

    assert!(LINE_COUNT * RAW_LINE_BYTES < MAX_LOG_BYTES);

    let root = TestDir::new("logcut-issue-52", "normal-masking-truncate");
    let logs = root.join("logs");
    prepare_log_directory(&logs);

    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .args([
            "sh",
            "-c",
            "yes TOKEN=x | head -n 700000; exit 52",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(52));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr
            .matches("command output exceeded 10 MiB and was truncated")
            .count(),
        1
    );

    let files = log_files(&logs);
    assert_eq!(files.len(), 1);
    let log = fs::read(&files[0]).unwrap();
    assert!(log.len() as u64 <= MAX_LOG_BYTES);
    assert!(!log
        .windows(b"TOKEN=x".len())
        .any(|value| value == b"TOKEN=x"));
    assert!(log
        .windows(b"[REDACTED]".len())
        .any(|value| value == b"[REDACTED]"));
    assert!(log
        .windows(TRUNCATION_MARKER.len())
        .any(|value| value == TRUNCATION_MARKER));
}
