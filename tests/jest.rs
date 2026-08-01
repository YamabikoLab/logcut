#![cfg(target_os = "linux")]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
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

fn run(profile: Option<&str>, body: &str) -> std::process::Output {
    let escaped = body
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    let root = temp_dir("jest");
    let mut command = Command::new(binary());
    command.env("LOGCUT_LOG_DIRECTORY", root.join("logs"));
    if let Some(profile) = profile {
        command.arg(format!("--profile={profile}"));
    }
    command
        .args(["sh", "-c", &format!("printf \"{escaped}\"; exit 1")])
        .output()
        .unwrap()
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn jest_profile_and_auto_detection_show_concise_failures() {
    let body = " FAIL  ./tests/math.test.ts\n  calculator\n    ✕ adds values (4 ms)\n\n  ● calculator › adds values\n\n    Expected: 4\n    Received: 5\n\nTest Suites: 1 failed, 2 passed, 3 total\nTests:       1 failed, 5 passed, 6 total\nSnapshots:   0 total\nTime:        1.234 s\nRan all test suites.\n";

    for profile in [Some("jest"), None] {
        let output = run(profile, body);
        let text = combined(&output);

        assert_eq!(output.status.code(), Some(1));
        assert!(text.contains("Failure summary (jest)"));
        assert!(text.contains(" FAIL  ./tests/math.test.ts"));
        assert!(text.contains("● calculator › adds values"));
        assert!(text.contains("Expected: 4"));
        assert!(text.contains("Received: 5"));
        assert!(text.contains("Test Suites: 1 failed, 2 passed, 3 total"));
        assert!(text.contains("Tests:       1 failed, 5 passed, 6 total"));
    }
}

#[test]
fn jest_runtime_errors_are_included_in_summary() {
    for cause in [
        "TypeError: Cannot read properties of undefined",
        "ReferenceError: value is not defined",
        "SyntaxError: Cannot use import statement outside a module",
        "Cannot find module 'missing-package'",
    ] {
        let body = format!(
            " FAIL  ./tests/setup.test.ts\n\n  ● Test suite failed to run\n\n    {cause}\n\nTest Suites: 1 failed, 1 total\nTests:       0 total\nSnapshots:   0 total\nTime:        0.123 s\nRan all test suites.\n"
        );
        let output = run(None, &body);
        let text = combined(&output);

        assert_eq!(output.status.code(), Some(1));
        assert!(text.contains("Failure summary (jest)"));
        assert!(text.contains("● Test suite failed to run"));
        assert!(text.contains(cause));
    }
}
