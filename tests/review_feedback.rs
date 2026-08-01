#![cfg(target_os = "linux")]

mod common;

use common::TestDir;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_logcut")
}

fn temp_dir(name: &str) -> TestDir {
    TestDir::new("logcut-review", name)
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn run(name: &str, profile: Option<&str>, body: &str) -> std::process::Output {
    let root = temp_dir(name);
    let mut command = Command::new(binary());
    command.env("LOGCUT_LOG_DIRECTORY", root.join("logs"));
    if let Some(profile) = profile {
        command.arg(format!("--profile={profile}"));
    }
    command.args(["sh", "-c", body]).output().unwrap()
}

fn process_exists(pid: i32) -> bool {
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn wait_for_file(path: &Path) {
    for _ in 0..100 {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {}", path.display());
}

#[test]
fn playwright_preserves_multiline_results_and_project_file_names() {
    let body = r#"printf '  1) [chromium] › tests/e2e/login.e2e.ts:12:3 › login

    Error: playwright boom

    Call log:
      - waiting for locator

  1 passed
  1 failed
  1 skipped
'; exit 1"#;
    let output = run("playwright-multiline", None, body);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(1));
    assert!(text.contains("Failure summary (playwright)"), "{text}");
    assert!(text.contains("login.e2e.ts:12:3"), "{text}");
    assert!(text.contains("1 passed; 1 failed; 1 skipped"), "{text}");
}

#[test]
fn playwright_avoids_aggregate_only_false_positive_and_honors_line_limit() {
    let negative = run(
        "playwright-negative",
        None,
        "printf '1 failed\ngeneric failure\n'; exit 1",
    );
    assert!(combined(&negative).contains("Failure summary (generic)"));

    let root = temp_dir("playwright-limit");
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
        .env("LOGCUT_SUMMARY_LINES", "5")
        .arg("--profile=playwright")
        .args([
            "sh",
            "-c",
            "printf '  1) [chromium] › tests/a.e2e.ts:1:1 › a\n\n    Error: boom\n\n    Call log:\n      - detail\n\n    attachment #1: trace\n      test-results/a/trace.zip\n\n  1 passed\n  1 failed\n'; exit 1",
        ])
        .output()
        .unwrap();
    let text = combined(&output);
    let summary = text
        .split("----- Failure summary (playwright) -----")
        .nth(1)
        .unwrap()
        .split("Full log:")
        .next()
        .unwrap();
    assert_eq!(
        summary
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count(),
        5
    );
    assert!(summary.contains("1 passed; 1 failed"));
}

#[test]
fn generic_fallback_and_numeric_validation_match_quiet_run() {
    let fallback = run(
        "generic-fallback",
        Some("phpunit"),
        "printf 'PHPUnit 12.0\nConfiguration could not be read\n'; exit 2",
    );
    assert!(combined(&fallback).contains("Configuration could not be read"));

    let root = temp_dir("oversized-number");
    let oversized = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
        .env(
            "LOGCUT_SUMMARY_LINES",
            "999999999999999999999999999999999999",
        )
        .args(["sh", "-c", "printf oversized; exit 4"])
        .output()
        .unwrap();
    let text = combined(&oversized);
    assert_eq!(oversized.status.code(), Some(4));
    assert!(text.contains("invalid LOGCUT_SUMMARY_LINES"));
    assert!(text.contains("oversized"));
}

#[test]
fn secure_log_failure_runs_command_directly() {
    let root = temp_dir("direct-fallback");
    let not_directory = root.join("not-directory");
    fs::write(&not_directory, "x").unwrap();
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", not_directory.join("logs"))
        .args(["sh", "-c", "printf direct-output; exit 23"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(23));
    assert!(text.contains("secure logging is unavailable"));
    assert!(text.contains("direct-output"));
}

#[test]
fn retained_logs_are_pruned_to_configured_count() {
    let root = temp_dir("retention");
    let logs = root.join("logs");
    fs::create_dir_all(&logs).unwrap();
    fs::set_permissions(&logs, fs::Permissions::from_mode(0o700)).unwrap();
    for index in 0..3 {
        fs::write(logs.join(format!("command.old-{index}.log")), "old").unwrap();
        thread::sleep(Duration::from_millis(10));
    }
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .env("LOGCUT_LOG_MAX_FILES", "2")
        .args(["sh", "-c", "printf new; exit 1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let count = fs::read_dir(logs)
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("command."))
        .count();
    assert_eq!(count, 2);
}

#[test]
fn hup_int_and_term_are_forwarded_and_process_group_is_cleaned_up() {
    for (name, signal, expected) in [
        ("HUP", libc::SIGHUP, 129),
        ("INT", libc::SIGINT, 130),
        ("TERM", libc::SIGTERM, 143),
    ] {
        let root = temp_dir(&format!("signal-{name}"));
        let child_file = root.join("child");
        let grandchild_file = root.join("grandchild");
        let child = Command::new(binary())
            .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
            .args([
                "sh",
                "-c",
                "echo $$ >\"$1\"; (trap 'exit 0' HUP INT TERM; while :; do sleep 1; done) & echo $! >\"$2\"; wait",
                "_",
                child_file.to_str().unwrap(),
                grandchild_file.to_str().unwrap(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        wait_for_file(&child_file);
        wait_for_file(&grandchild_file);
        unsafe {
            assert_eq!(libc::kill(child.id() as i32, signal), 0);
        }
        let output = child.wait_with_output().unwrap();
        let text = combined(&output);
        assert_eq!(output.status.code(), Some(expected), "{name}: {text}");
        assert!(text.contains(&format!("exit {expected}")), "{name}: {text}");

        let child_pid: i32 = fs::read_to_string(&child_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let grandchild_pid: i32 = fs::read_to_string(&grandchild_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        for _ in 0..50 {
            if !process_exists(child_pid) && !process_exists(grandchild_pid) {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!process_exists(child_pid), "{name}: child remained");
        assert!(
            !process_exists(grandchild_pid),
            "{name}: grandchild remained"
        );
    }
}
