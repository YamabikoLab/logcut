#![cfg(target_os = "linux")]

mod common;

use common::TestDir;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_logcut")
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn write_fake_command(directory: &Path, name: &str, body: &str) {
    let path = directory.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn run_fake(name: &str, arguments: &[&str], body: &str) -> Output {
    let root = TestDir::new("logcut-issue-34-regressions", name);
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_fake_command(&bin, name, body);

    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let path = format!("{}:{}", bin.display(), inherited_path.to_string_lossy());
    Command::new(binary())
        .env("PATH", path)
        .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
        .arg(name)
        .args(arguments)
        .output()
        .unwrap()
}

#[test]
fn quoted_and_json_secrets_are_redacted_from_summary() {
    let output = run_fake(
        "git",
        &["push"],
        r#"printf '%s\n' 'password="alpha beta" token='"'"'gamma delta'"'"'' '{"token":"json secret","password": "other secret"}' 'fatal: Authentication failed' >&2; exit 128"#,
    );
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(128), "{text}");
    for secret in ["alpha beta", "gamma delta", "json secret", "other secret"] {
        assert!(!text.contains(secret), "{text}");
    }
    assert!(text.contains("[REDACTED]"));
}

#[test]
fn git_path_query_options_are_not_misdetected_as_transfers() {
    for arguments in [
        vec!["--html-path", "push"],
        vec!["--man-path", "fetch"],
        vec!["--info-path", "pull"],
    ] {
        let output = run_fake("git", &arguments, "printf '%s\\n' '/usr/share/git'");
        let text = combined(&output);
        assert!(output.status.success(), "{text}");
        assert!(!text.contains("Running: git push"), "{text}");
        assert!(!text.contains("Running: git fetch"), "{text}");
        assert!(!text.contains("Running: git pull"), "{text}");
        assert!(!text.contains("Result: no ref updates reported."), "{text}");
    }
}
