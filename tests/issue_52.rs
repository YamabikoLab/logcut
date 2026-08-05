#![cfg(target_os = "linux")]

mod common;

use common::TestDir;
use std::fs;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;

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

fn directory_bytes(directory: &std::path::Path) -> u64 {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}

#[test]
fn oversized_output_is_truncated_without_changing_exit_code() {
    let root = TestDir::new("logcut-issue-52", "truncate");
    let logs = root.join("logs");
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .args([
            "sh",
            "-c",
            "yes 0123456789abcdef | head -c 11534336; exit 52",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(52));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("command output exceeded 10 MiB and was truncated"));

    let files = log_files(&logs);
    assert_eq!(files.len(), 1);
    let metadata = fs::metadata(&files[0]).unwrap();
    assert!(metadata.len() <= MAX_LOG_BYTES);

    let log = fs::read_to_string(&files[0]).unwrap();
    assert!(log.contains("[logcut: command output truncated at 10 MiB]"));
}

#[test]
fn capture_storage_stays_bounded_while_the_foreground_command_is_running() {
    let root = TestDir::new("logcut-issue-52", "running-storage");
    let logs = root.join("logs");
    let ready = root.join("ready");
    let release = root.join("release");
    fs::create_dir_all(&logs).unwrap();

    let mut child = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .env("READY", &ready)
        .env("RELEASE", &release)
        .args([
            "sh",
            "-c",
            "yes x | head -c 33554432; : > \"$READY\"; while [ ! -e \"$RELEASE\" ]; do sleep 0.02; done; exit 52",
        ])
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(ready.exists(), "foreground command did not reach the inspection point");
    assert!(directory_bytes(&logs) <= MAX_LOG_BYTES);

    fs::write(&release, b"go").unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(52));
    assert!(directory_bytes(&logs) <= MAX_LOG_BYTES);
}

#[test]
fn long_lived_background_output_does_not_grow_disk_storage() {
    let root = TestDir::new("logcut-issue-52", "background-storage");
    let logs = root.join("logs");
    let background_pid = root.join("background.pid");
    fs::create_dir_all(&logs).unwrap();

    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .env("BACKGROUND_PID", &background_pid)
        .args([
            "sh",
            "-c",
            "(while true; do yes background | head -c 1048576; done) & echo $! > \"$BACKGROUND_PID\"; exit 52",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(52));

    let process_id = fs::read_to_string(&background_pid)
        .unwrap()
        .trim()
        .parse::<libc::pid_t>()
        .unwrap();
    let before = directory_bytes(&logs);
    thread::sleep(Duration::from_millis(500));
    let after = directory_bytes(&logs);

    assert!(before <= MAX_LOG_BYTES);
    assert!(after <= MAX_LOG_BYTES);
    assert_eq!(after, before, "background output continued consuming disk space");

    unsafe {
        libc::kill(process_id, libc::SIGTERM);
    }
}
