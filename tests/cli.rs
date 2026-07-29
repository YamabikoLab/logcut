#![cfg(target_os = "linux")]

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

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

fn run(name: &str, profile: Option<&str>, script: &str) -> std::process::Output {
    let root = temp_dir(name);
    let logs = root.join("logs");
    let mut command = Command::new(binary());
    command.env("LOGCUT_LOG_DIRECTORY", &logs);
    if let Some(profile) = profile {
        command.arg(format!("--profile={profile}"));
    }
    command.args(["sh", "-c", script]).output().unwrap()
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn success_suppresses_output_and_removes_log() {
    let root = temp_dir("success");
    let logs = root.join("logs");
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .args(["sh", "-c", "printf noisy-output"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert!(output.status.success());
    assert!(text.contains("Running: sh [2 args]"));
    assert!(text.contains("PASS ("));
    assert!(!text.contains("noisy-output"));
    assert_eq!(
        fs::metadata(&logs).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert!(fs::read_dir(logs).unwrap().next().is_none());
}

#[test]
fn failure_keeps_exit_code_summary_and_full_log() {
    let output = run("failure", None, "printf failure-body; exit 37");
    assert_eq!(output.status.code(), Some(37));
    let text = combined(&output);
    assert!(text.contains("failure-body"));
    assert!(text.contains("FAIL ("));
    assert!(text.contains("exit 37"));
    assert!(text.contains("Full log:"));
}

#[test]
fn stdin_is_forwarded_without_success_output_leaking() {
    let root = temp_dir("stdin");
    let logs = root.join("logs");
    let destination = root.join("stdin.txt");
    let mut child = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .args([
            "sh",
            "-c",
            "cat >\"$1\"",
            "_",
            destination.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"alpha\nbeta\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(fs::read_to_string(destination).unwrap(), "alpha\nbeta\n");
    assert!(!combined(&output).contains("alpha"));
}

#[test]
fn arguments_are_not_printed() {
    let secret = "token-value-that-must-not-appear";
    let root = temp_dir("arguments");
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
        .args(["sh", "-c", "test \"$1\" = \"$2\"", "_", secret, secret])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!combined(&output).contains(secret));
}

#[test]
fn control_sequences_are_removed_from_summary() {
    let output = run(
        "controls",
        Some("generic"),
        r"printf '\033[31mred\033[0m\r\n\033]8;;https://example.invalid\alink\033]8;;\033\\\n'; exit 1",
    );
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(1));
    assert!(text.contains("red"));
    assert!(text.contains("link"));
    assert!(!text.contains('\u{1b}'));
    assert!(!text.contains("https://example.invalid"));
}

#[test]
fn invalid_numeric_settings_use_defaults() {
    let root = temp_dir("numeric");
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
        .env("LOGCUT_SUMMARY_LINES", "bad")
        .env("LOGCUT_MAX_ERRORS", "0")
        .args(["sh", "-c", "printf numeric; exit 3"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(3));
    assert!(text.contains("invalid LOGCUT_SUMMARY_LINES"));
    assert!(text.contains("numeric"));
}

#[test]
fn all_profiles_extract_expected_failure() {
    let fixtures = [
        (
            "vitest",
            " FAIL  tests/a.test.ts\nAssertionError: vitest boom\n⎯\n",
            "vitest boom",
        ),
        (
            "prettier",
            "[warn] src/a.ts\nCode style issues found\n",
            "[warn] src/a.ts",
        ),
        (
            "eslint",
            "src/a.ts\n  1:2  error  eslint boom  rule\n✖ 1 problem\n",
            "eslint boom",
        ),
        (
            "typescript",
            "src/a.ts(1,2): error TS1234: typescript boom\n",
            "typescript boom",
        ),
        (
            "phpunit",
            "PHPUnit 12.0\nThere was 1 failure:\n1) A::b\nphpunit boom\nTests: 1\n",
            "phpunit boom",
        ),
        (
            "phpstan",
            "phpstan boom\n [ERROR] Found 1 error\n",
            "phpstan boom",
        ),
        (
            "php-lint",
            "PHP Parse error: boom\nErrors parsing a.php\n",
            "PHP Parse error",
        ),
        (
            "contract",
            "Contract check failed:\ncontract boom\n",
            "contract boom",
        ),
        (
            "vite",
            "error during build:\nRollupError: vite boom\n",
            "vite boom",
        ),
        (
            "composer",
            "Script test returned with error code 1\ncomposer boom\n",
            "returned with error code",
        ),
        ("generic", "generic boom\n", "generic boom"),
    ];

    for (profile, body, expected) in fixtures {
        let escaped = body
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        let output = run(
            profile,
            Some(profile),
            &format!("printf \"{escaped}\"; exit 1"),
        );
        let text = combined(&output);
        assert_eq!(output.status.code(), Some(1), "profile {profile}");
        assert!(
            text.contains(&format!("Failure summary ({profile})")),
            "{text}"
        );
        assert!(text.contains(expected), "profile {profile}: {text}");
    }
}

#[test]
fn playwright_profile_and_auto_detection_are_concise() {
    let body = "  1) [chromium] › tests/e2e/admin.smoke.spec.ts:12:3 › saves the block\n\n    Error: playwright boom\n\n    Call log:\n      - waiting for locator\n\n    attachment #1: trace (application/zip)\n      test-results/admin-smoke/trace.zip\n\n  1 passed, 1 failed, 1 skipped\n";
    let escaped = body
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    for profile in [Some("playwright"), None] {
        let output = run(
            "playwright",
            profile,
            &format!("printf \"{escaped}\"; exit 1"),
        );
        let text = combined(&output);
        assert!(text.contains("Failure summary (playwright)"));
        assert!(text.contains("admin.smoke.spec.ts:12:3"));
        assert!(text.contains("Error: playwright boom"));
        assert!(text.contains("Call log: - waiting for locator"));
        assert!(text.contains("test-results/admin-smoke/trace.zip"));
        assert!(text.contains("1 passed; 1 failed; 1 skipped"));
    }
}
