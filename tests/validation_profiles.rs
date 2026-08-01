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
        "logcut-validation-{name}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn run(name: &str, profile: Option<&str>, body: &str, exit_code: i32) -> std::process::Output {
    let escaped = body
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    let root = temp_dir(name);
    let mut command = Command::new(binary());
    command.env("LOGCUT_LOG_DIRECTORY", root.join("logs"));
    if let Some(profile) = profile {
        command.arg(format!("--profile={profile}"));
    }
    command
        .args([
            "sh",
            "-c",
            &format!("printf \"{escaped}\"; exit {exit_code}"),
        ])
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
fn phpcs_profile_and_auto_detection_show_file_errors() {
    let body = "FILE: /app/example.php\n----------------------------------------------------------------------\nFOUND 2 ERRORS AFFECTING 2 LINES\n----------------------------------------------------------------------\n 10 | ERROR | Missing file doc comment\n 20 | ERROR | Expected 1 space after comma\n----------------------------------------------------------------------\n\nPHPCS REPORT SUMMARY\n----------------------------------------------------------------------\nFILE                                                  ERRORS  WARNINGS\n----------------------------------------------------------------------\n/app/example.php                                      2       0\n----------------------------------------------------------------------\nA TOTAL OF 2 ERRORS AND 0 WARNINGS WERE FOUND IN 1 FILE\n";

    for profile in [Some("phpcs"), None] {
        let output = run("phpcs", profile, body, 1);
        let text = combined(&output);

        assert_eq!(output.status.code(), Some(1));
        assert!(text.contains("Failure summary (phpcs)"), "{text}");
        assert!(text.contains("FILE: /app/example.php"), "{text}");
        assert!(text.contains("Missing file doc comment"), "{text}");
        assert!(text.contains("A TOTAL OF 2 ERRORS"), "{text}");
    }
}

#[test]
fn phpcbf_exit_one_is_accepted_when_all_errors_are_fixed() {
    let body = "PHPCBF RESULT SUMMARY\n----------------------------------------------------------------------\nFILE                                                  FIXED  REMAINING\n----------------------------------------------------------------------\n/app/example.php                                      23     0\n----------------------------------------------------------------------\nA TOTAL OF 23 ERRORS WERE FIXED IN 1 FILE\n";

    for profile in [Some("phpcbf"), None] {
        let output = run("phpcbf-fixed", profile, body, 1);
        let text = combined(&output);

        assert!(output.status.success(), "{text}");
        assert!(text.contains("PASS ("), "{text}");
        assert!(!text.contains("Failure summary"), "{text}");
    }
}

#[test]
fn phpcbf_exit_one_keeps_failure_when_summary_has_remaining_errors() {
    let body = "PHPCBF RESULT SUMMARY\n----------------------------------------------------------------------\nFILE                                                  FIXED  REMAINING\n----------------------------------------------------------------------\n/app/example.php                                      23     1\n----------------------------------------------------------------------\nA TOTAL OF 23 ERRORS WERE FIXED IN 1 FILE\n";

    for profile in [Some("phpcbf"), None] {
        let output = run("phpcbf-remaining-exit-one", profile, body, 1);
        let text = combined(&output);

        assert_eq!(output.status.code(), Some(1));
        assert!(text.contains("Failure summary (phpcbf)"), "{text}");
        assert!(text.contains("A TOTAL OF 23 ERRORS WERE FIXED"), "{text}");
    }
}

#[test]
fn phpcbf_keeps_failure_when_errors_remain() {
    let body = "PHPCBF RESULT SUMMARY\n----------------------------------------------------------------------\nFILE                                                  FIXED  REMAINING\n----------------------------------------------------------------------\n/app/example.php                                      2      1\n----------------------------------------------------------------------\nA TOTAL OF 2 ERRORS WERE FIXED IN 1 FILE\nPHPCBF FAILED TO FIX 1 ERROR\n";
    let output = run("phpcbf-remaining", None, body, 2);
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(2));
    assert!(text.contains("Failure summary (phpcbf)"), "{text}");
    assert!(text.contains("FAILED TO FIX 1 ERROR"), "{text}");
}

#[test]
fn webpack_profile_and_auto_detection_show_build_error() {
    let body = "assets by status 1.2 KiB [cached] 1 asset\nERROR in ./src/index.js 1:0\nModule parse failed: Unexpected token (1:0)\nYou may need an appropriate loader to handle this file type\nwebpack compiled with 1 error in 412 ms\n";

    for profile in [Some("webpack"), None] {
        let output = run("webpack", profile, body, 1);
        let text = combined(&output);

        assert_eq!(output.status.code(), Some(1));
        assert!(text.contains("Failure summary (webpack)"), "{text}");
        assert!(text.contains("ERROR in ./src/index.js"), "{text}");
        assert!(text.contains("Module parse failed"), "{text}");
        assert!(text.contains("webpack compiled with 1 error"), "{text}");
    }
}
