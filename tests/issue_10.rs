#![cfg(target_os = "linux")]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
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

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn large_failure_log_reads_only_a_bounded_tail() {
    let root = temp_dir("large-tail");
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
        .arg("--profile=generic")
        .args([
            "sh",
            "-c",
            "head -c 2097152 /dev/zero | tr '\\0' x; printf '\\nTAIL-MARKER\\n'; exit 5",
        ])
        .output()
        .unwrap();

    let text = combined(&output);
    assert_eq!(output.status.code(), Some(5));
    assert!(text.contains("TAIL-MARKER"));
    assert!(text.len() < 10_000, "summary unexpectedly retained the full log");
}

#[test]
fn ignored_forwarded_signal_is_escalated() {
    let root = temp_dir("signal-escalation");
    let mut child = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
        .args([
            "sh",
            "-c",
            "trap '' TERM; while :; do sleep 1; done",
        ])
        .spawn()
        .unwrap();

    thread::sleep(Duration::from_millis(200));
    let start = Instant::now();
    // SAFETY: The PID belongs to the child process spawned above.
    assert_eq!(unsafe { kill(child.id() as i32, 15) }, 0);
    let status = child.wait().unwrap();

    assert_eq!(status.code(), Some(143));
    assert!(start.elapsed() < Duration::from_secs(4));
}

#[test]
fn vitest_summary_respects_the_final_line_limit() {
    let root = temp_dir("summary-limit");
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
        .env("LOGCUT_SUMMARY_LINES", "3")
        .arg("--profile=vitest")
        .args([
            "sh",
            "-c",
            "printf ' FAIL  tests/a.test.ts\\nline 1\\nline 2\\nline 3\\nline 4\\n⎯\\n'; exit 1",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    let summary: Vec<&str> = stderr
        .lines()
        .skip_while(|line| !line.starts_with("----- Failure summary"))
        .skip(1)
        .take_while(|line| !line.starts_with("Full log:"))
        .filter(|line| !line.is_empty())
        .collect();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(summary.len(), 3, "{stderr}");
    assert_eq!(summary.last(), Some(&"[additional summary lines omitted]"));
}

#[test]
fn unreadable_failure_log_preserves_child_exit_code() {
    let root = temp_dir("unreadable-log");
    let logs = root.join("logs");
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .args([
            "sh",
            "-c",
            "rm -f \"$LOGCUT_LOG_DIRECTORY\"/command.*.log; exit 37",
        ])
        .output()
        .unwrap();

    let text = combined(&output);
    assert_eq!(output.status.code(), Some(37));
    assert!(text.contains("failure summary could not be generated"));
    assert!(text.contains("Full log:"));
}
