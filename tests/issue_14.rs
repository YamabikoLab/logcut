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
        "logcut-issue-14-{name}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn run(name: &str, script: &str) -> std::process::Output {
    Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", temp_dir(name).join("logs"))
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
