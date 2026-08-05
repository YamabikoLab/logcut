#![cfg(target_os = "linux")]

mod common;

use common::TestDir;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_logcut")
}

fn fake_command(root: &Path, name: &str, body: &str, exit_code: i32) -> PathBuf {
    let path = root.join(name);
    let script = format!("#!/bin/sh\ncat <<'LOGCUT_OUTPUT'\n{body}LOGCUT_OUTPUT\nexit {exit_code}\n");
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn run_command(name: &str, executable: &str, arguments: &[&str], body: &str, exit_code: i32) -> Output {
    let root = TestDir::new("logcut-issue-62", name);
    let command_path = fake_command(&root, executable, body, exit_code);
    let mut command = Command::new(binary());
    command
        .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
        .arg(command_path);
    command.args(arguments).output().unwrap()
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn npm_ci_uses_npm_install_profile_and_preserves_failure_exit_code() {
    let body = "npm error code ERESOLVE\nnpm error While resolving: example@1.0.0\nnpm error Found: react@18.3.1\nnpm error node_modules/react\nnpm error Could not resolve dependency:\nnpm error peer react@\"^17\" from legacy-plugin@2.0.0\nnpm error Conflicting peer dependency: react@17.0.2\n";
    let output = run_command("npm-ci-failure", "npm", &["ci"], body, 42);
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(42), "{text}");
    assert!(text.contains("Running: npm ci"), "{text}");
    assert!(text.contains("Failure summary (npm-install)"), "{text}");
    assert!(text.contains("Code: ERESOLVE"), "{text}");
    assert!(
        text.contains("Cause: dependency tree could not be resolved"),
        "{text}"
    );
    assert!(text.contains("While resolving: example@1.0.0"), "{text}");
    assert!(
        text.contains("peer react@\"^17\" from legacy-plugin@2.0.0"),
        "{text}"
    );
}

#[test]
fn npm_install_commands_are_detected_from_the_subcommand() {
    for (name, arguments) in [
        ("npm-ci", vec!["ci"]),
        ("npm-install", vec!["install"]),
        ("npm-i", vec!["i", "example@1.0.0"]),
        (
            "npm-lock-only",
            vec!["install", "--package-lock-only"],
        ),
        (
            "npm-workspace",
            vec!["--workspace", "packages/example", "install"],
        ),
    ] {
        let output = run_command(
            name,
            "npm",
            &arguments,
            "npm error code ETARGET\nnpm error No matching version found for example@9.9.9.\n",
            1,
        );
        let text = combined(&output);

        assert_eq!(output.status.code(), Some(1), "{name}: {text}");
        assert!(
            text.contains("Failure summary (npm-install)"),
            "{name}: {text}"
        );
        assert!(text.contains("Code: ETARGET"), "{name}: {text}");
    }
}

#[test]
fn npm_install_success_keeps_only_compact_counts() {
    let body = "npm warn deprecated old-package@1.0.0: no longer supported\nnpm warn ERESOLVE overriding peer dependency\nadded 12 packages, removed 1 package, and changed 2 packages in 3s\n3 vulnerabilities (1 moderate, 2 high)\n";
    let output = run_command(
        "npm-install-success",
        "npm",
        &["install", "--package-lock-only"],
        body,
        0,
    );
    let text = combined(&output);

    assert!(output.status.success(), "{text}");
    assert!(text.contains("Running: npm install"), "{text}");
    assert!(text.contains("PASS ("), "{text}");
    assert!(text.contains("added 12 packages"), "{text}");
    assert!(text.contains("Peer dependency warnings: 1"), "{text}");
    assert!(text.contains("Deprecated warnings: 1"), "{text}");
    assert!(
        text.contains("3 vulnerabilities (1 moderate, 2 high)"),
        "{text}"
    );
    assert!(!text.contains("old-package@1.0.0"), "{text}");
}

#[test]
fn npm_run_npm_ls_and_npx_remain_outside_the_profile() {
    for (name, executable, arguments) in [
        ("npm-run", "npm", vec!["run", "build"]),
        ("npm-ls", "npm", vec!["ls"]),
        ("npx", "npx", vec!["playwright", "test"]),
    ] {
        let output = run_command(
            name,
            executable,
            &arguments,
            "npm error code ERESOLVE\nnpm error Could not resolve dependency:\n",
            1,
        );
        let text = combined(&output);

        assert_eq!(output.status.code(), Some(1), "{name}: {text}");
        assert!(
            !text.contains("Failure summary (npm-install)"),
            "{name}: {text}"
        );
    }
}

#[test]
fn npm_install_profile_can_be_selected_explicitly() {
    let root = TestDir::new("logcut-issue-62", "explicit-profile");
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
        .args([
            "--profile=npm-install",
            "sh",
            "-c",
            "printf 'npm error code E401\\nnpm error Unable to authenticate\\n'; exit 7",
        ])
        .output()
        .unwrap();
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(7), "{text}");
    assert!(text.contains("Failure summary (npm-install)"), "{text}");
    assert!(text.contains("Code: E401"), "{text}");
    assert!(text.contains("Cause: registry authentication failed"), "{text}");
}
