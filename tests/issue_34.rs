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
    let root = TestDir::new("logcut-issue-34", name);
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
fn docker_build_success_keeps_image_and_removes_progress() {
    let output = run_fake(
        "docker",
        &["build", "."],
        "printf '%s\\n' '#1 [internal] load build definition from Dockerfile' '#1 DONE 0.0s' '#8 naming to docker.io/library/app:latest done' >&2",
    );
    let text = combined(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("Running: docker build"));
    assert!(text.contains("PASS ("));
    assert!(text.contains("Image: docker.io/library/app:latest"));
    assert!(!text.contains("load build definition"));
}

#[test]
fn docker_compose_failure_uses_dedicated_summary() {
    let output = run_fake(
        "docker",
        &["compose", "build", "worker"],
        "printf '%s\\n' '#9 [worker 4/4] RUN cargo build' 'Dockerfile:18' 'error: linker failed' 'ERROR: failed to solve: process exited with exit code: 1' >&2; exit 1",
    );
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("Running: docker compose build"));
    assert!(text.contains("Failure summary (docker-build)"));
    assert!(text.contains("Service: worker"));
    assert!(text.contains("Dockerfile:18"));
    assert!(text.contains("exit code: 1"));
    assert!(text.contains("Full log:"));
}

#[test]
fn git_push_success_keeps_remote_ref_and_commit_range() {
    let output = run_fake(
        "git",
        &["push", "origin", "main"],
        "printf '%s\\n' 'To github.com:YamabikoLab/logcut.git' '   1111111..2222222  main -> main' >&2",
    );
    let text = combined(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("Running: git push"));
    assert!(text.contains("Remote: github.com:YamabikoLab/logcut.git"));
    assert!(text.contains("1111111..2222222"));
    assert!(text.contains("main -> main"));
}

#[test]
fn git_failure_is_classified_and_secrets_are_redacted() {
    let output = run_fake(
        "git",
        &["push"],
        "printf '%s\\n' 'Authorization: Bearer secret-value' 'fatal: unable to access https://user:password@example.invalid/repo: Authentication failed' >&2; exit 128",
    );
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(128), "{text}");
    assert!(text.contains("Failure summary (git-transfer)"));
    assert!(text.contains("Cause: authentication or repository permission failed"));
    assert!(!text.contains("secret-value"));
    assert!(!text.contains("user:password"));
    assert!(text.contains("[REDACTED]"));
}
