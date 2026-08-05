#![cfg(target_os = "linux")]

mod common;

use common::{prepare_log_directory, TestDir};
use std::fs;
use std::process::{Command, Stdio};
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

fn descriptor_bytes(process_id: libc::pid_t, descriptor: u8) -> u64 {
    fs::metadata(format!("/proc/{process_id}/fd/{descriptor}"))
        .unwrap()
        .len()
}

fn process_is_alive(process_id: libc::pid_t) -> bool {
    if unsafe { libc::kill(process_id, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[test]
fn oversized_output_is_truncated_without_changing_exit_code() {
    let root = TestDir::new("logcut-issue-52", "truncate");
    let logs = root.join("logs");
    prepare_log_directory(&logs);

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
    let foreground_pid = root.join("foreground.pid");
    let ready = root.join("ready");
    let release = root.join("release");
    prepare_log_directory(&logs);
    let baseline = directory_bytes(&logs);

    let mut child = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .env("FOREGROUND_PID", &foreground_pid)
        .env("READY", &ready)
        .env("RELEASE", &release)
        .args([
            "sh",
            "-c",
            "echo $$ > \"$FOREGROUND_PID\"; yes x | head -c 33554432; : > \"$READY\"; while [ ! -e \"$RELEASE\" ]; do sleep 0.02; done; exit 52",
        ])
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        ready.exists(),
        "foreground command did not reach the inspection point"
    );

    let process_id = fs::read_to_string(&foreground_pid)
        .unwrap()
        .trim()
        .parse::<libc::pid_t>()
        .unwrap();
    assert!(descriptor_bytes(process_id, 1) <= MAX_LOG_BYTES);
    assert!(directory_bytes(&logs).saturating_sub(baseline) <= MAX_LOG_BYTES);

    fs::write(&release, b"go").unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(52));
    assert!(directory_bytes(&logs).saturating_sub(baseline) <= MAX_LOG_BYTES);
}

#[test]
fn infinite_background_output_does_not_delay_exit_or_change_the_final_log() {
    let root = TestDir::new("logcut-issue-52", "background-storage");
    let logs = root.join("logs");
    let background_pid = root.join("background.pid");
    prepare_log_directory(&logs);
    let baseline = directory_bytes(&logs);

    let mut child = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .env("BACKGROUND_PID", &background_pid)
        .args([
            "sh",
            "-c",
            "(sleep 0.2; exec yes background) & echo $! > \"$BACKGROUND_PID\"; exit 52",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("logcut did not exit after the foreground command completed");
        }
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.code(), Some(52));

    let process_id = fs::read_to_string(&background_pid)
        .unwrap()
        .trim()
        .parse::<libc::pid_t>()
        .unwrap();
    let before = directory_bytes(&logs).saturating_sub(baseline);
    let files = log_files(&logs);
    assert_eq!(files.len(), 1);
    let contents_before = fs::read(&files[0]).unwrap();

    thread::sleep(Duration::from_millis(500));

    let after = directory_bytes(&logs).saturating_sub(baseline);
    let contents_after = fs::read(&files[0]).unwrap();
    assert!(before <= MAX_LOG_BYTES);
    assert_eq!(after, before, "background output changed retained storage");
    assert_eq!(
        contents_after, contents_before,
        "background output changed the finalized log"
    );

    if process_is_alive(process_id) {
        unsafe {
            libc::kill(process_id, libc::SIGTERM);
        }
    }
}

#[test]
fn redirected_background_process_can_continue_after_logcut_exits() {
    let root = TestDir::new("logcut-issue-52", "redirected-background");
    let logs = root.join("logs");
    let marker = root.join("background-finished");
    prepare_log_directory(&logs);

    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .env("BACKGROUND_MARKER", &marker)
        .args([
            "sh",
            "-c",
            "(sleep 0.2; printf finished > \"$BACKGROUND_MARKER\") >/dev/null 2>&1 </dev/null & exit 0",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());

    let deadline = Instant::now() + Duration::from_secs(2);
    while !marker.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        marker.exists(),
        "redirected background process did not continue after logcut exit"
    );
}
