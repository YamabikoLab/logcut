#![cfg(target_os = "linux")]

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
        "logcut-stylelint-test-{name}-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn run(name: &str, profile: Option<&str>, body: &str) -> std::process::Output {
    let root = temp_dir(name);
    let escaped = body
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    let mut command = Command::new(binary());
    command.env("LOGCUT_LOG_DIRECTORY", root.join("logs"));
    if let Some(profile) = profile {
        command.arg(format!("--profile={profile}"));
    }
    command
        .args(["sh", "-c", &format!("printf \"{escaped}\"; exit 2")])
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
fn explicit_profile_summarizes_css_and_scss_problems() {
    let body = "src/blocks/notice/editor.scss\n  1:40  ✖  Selector should use lowercase  selector-class-pattern\n\nsrc/blocks/notice/style.css\n  8:2  ✖  Unexpected duplicate property  declaration-block-no-duplicate-properties\n\n✖ 2 problems (2 errors, 0 warnings)\n";
    let output = run("explicit", Some("stylelint"), body);
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(2));
    assert!(text.contains("Failure summary (stylelint)"));
    assert!(text.contains("src/blocks/notice/editor.scss"));
    assert!(text.contains("1:40"));
    assert!(text.contains("src/blocks/notice/style.css"));
    assert!(text.contains("✖ 2 problems"));
}

#[test]
fn auto_detects_stylelint_after_fix_leaves_a_problem() {
    let body = "> wp-scripts lint-style \"src/**/*.{css,scss}\" --fix\n\nsrc/blocks/notice/editor.scss\n  4:1  ✖  Unexpected unknown property  property-no-unknown\n\n✖ 1 problem (1 error, 0 warnings)\n";
    let output = run("auto-fix", None, body);
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(2));
    assert!(text.contains("Failure summary (stylelint)"));
    assert!(text.contains("src/blocks/notice/editor.scss"));
    assert!(text.contains("property-no-unknown"));
}
