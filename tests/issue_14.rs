#![cfg(target_os = "linux")]

mod common;

use common::TestDir;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_logcut")
}

fn temp_dir(name: &str) -> TestDir {
    TestDir::new("logcut-issue-14", name)
}

fn run(name: &str, script: &str) -> std::process::Output {
    let root = temp_dir(name);
    Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
        .arg("--profile=generic")
        .args(["sh", "-c", script])
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
fn removes_unsafe_control_characters_but_preserves_lines_and_tabs() {
    let output = run(
        "control-characters",
        r"printf 'alpha\000\001\007\010\013\014\177beta\tgamma\n'; exit 1",
    );
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(1));
    assert!(text.contains("alphabeta\tgamma"), "{text:?}");
    assert!(!text
        .chars()
        .any(|character| { character.is_control() && character != '\n' && character != '\t' }));
}

#[test]
fn removes_supported_terminal_escape_sequences() {
    let output = run(
        "escape-sequences",
        r"printf 'before\033Pprivate-data\033\\after\nnext\033(0line\033cend\n'; exit 1",
    );
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(1));
    assert!(text.contains("beforeafter"), "{text:?}");
    assert!(text.contains("nextlineend"), "{text:?}");
    assert!(!text.contains("private-data"));
    assert!(!text.contains('\u{1b}'));
}

#[test]
fn does_not_treat_bel_as_a_dcs_terminator() {
    let output = run(
        "dcs-bel",
        r"printf 'before\033Psecret\007leaked\033\\after\n'; exit 1",
    );
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(1));
    assert!(text.contains("beforeafter"), "{text:?}");
    assert!(!text.contains("secret"));
    assert!(!text.contains("leaked"));
}
