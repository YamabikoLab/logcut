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
fn docker_compose_success_keeps_built_services() {
    let output = run_fake(
        "docker",
        &["compose", "--project-name", "sample", "build", "web", "worker"],
        "printf '%s\\n' 'web Built' 'worker Built' >&2",
    );
    let text = combined(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("Running: docker compose build"));
    assert!(text.contains("Service built: web"));
    assert!(text.contains("Service built: worker"));
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
fn docker_copy_source_missing_is_summarized() {
    let output = run_fake(
        "docker",
        &["build", "."],
        r#"printf '%s\n' '#7 [3/4] COPY missing.txt /app/' 'Dockerfile:12' 'ERROR: failed to solve: failed to compute cache key: "/missing.txt": not found' >&2; exit 1"#,
    );
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("Instruction: COPY missing.txt /app/"));
    assert!(text.contains("Dockerfile:12"));
    assert!(text.contains("not found"));
}

#[test]
fn docker_package_install_failure_is_summarized() {
    let output = run_fake(
        "docker",
        &["build", "."],
        "printf '%s\\n' '#6 [2/3] RUN apt-get update && apt-get install -y missing-package' 'E: Unable to locate package missing-package' 'ERROR: failed to solve: process exited with exit code: 100' >&2; exit 100",
    );
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(100), "{text}");
    assert!(text.contains("Instruction: RUN apt-get update"));
    assert!(text.contains("Unable to locate package missing-package"));
    assert!(text.contains("exit code: 100"));
}

#[test]
fn docker_compose_identifies_only_failed_service() {
    let output = run_fake(
        "docker",
        &["compose", "build"],
        r#"printf '%s\n' 'web Built' '#12 [worker 5/5] RUN cargo build' 'error: could not compile worker' 'ERROR: service "worker" failed to build' >&2; exit 1"#,
    );
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("Service: worker"));
    assert!(!text.contains("Service: web"));
}

#[test]
fn docker_compose_non_build_commands_are_not_misdetected() {
    for arguments in [
        vec!["compose", "run", "build"],
        vec!["compose", "exec", "web", "build"],
        vec!["compose", "--project-name", "build", "up"],
    ] {
        let output = run_fake("docker", &arguments, "printf '%s\\n' 'original output'");
        let text = combined(&output);
        assert!(output.status.success(), "{text}");
        assert!(!text.contains("Running: docker compose build"), "{text}");
        assert!(!text.contains("Docker build completed successfully."), "{text}");
    }
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
fn git_pull_fast_forward_success_is_summarized() {
    let output = run_fake(
        "git",
        &["pull", "--ff-only"],
        "printf '%s\\n' 'From github.com:YamabikoLab/logcut' 'Updating 1111111..2222222' 'Fast-forward' >&2",
    );
    let text = combined(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("Running: git pull"));
    assert!(text.contains("Remote: github.com:YamabikoLab/logcut"));
    assert!(text.contains("Updating 1111111..2222222"));
    assert!(text.contains("Fast-forward"));
}

#[test]
fn git_fetch_with_and_without_updates_is_summarized() {
    let updated = run_fake(
        "git",
        &["fetch", "origin"],
        "printf '%s\\n' 'From github.com:YamabikoLab/logcut' '   1111111..2222222  main -> origin/main' >&2",
    );
    let updated_text = combined(&updated);
    assert!(updated.status.success(), "{updated_text}");
    assert!(updated_text.contains("1111111..2222222"));

    let unchanged = run_fake("git", &["fetch", "origin"], ":");
    let unchanged_text = combined(&unchanged);
    assert!(unchanged.status.success(), "{unchanged_text}");
    assert!(unchanged_text.contains("Result: no ref updates reported."));
}

#[test]
fn git_non_fast_forward_is_classified() {
    let output = run_fake(
        "git",
        &["push", "origin", "main"],
        "printf '%s\\n' ' ! [rejected] main -> main (non-fast-forward)' 'error: failed to push some refs' >&2; exit 1",
    );
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("Cause: non-fast-forward update rejected"));
    assert!(text.contains("[rejected]"));
}

#[test]
fn git_missing_remote_is_classified() {
    let output = run_fake(
        "git",
        &["fetch", "missing"],
        "printf '%s\\n' 'fatal: repository not found' >&2; exit 128",
    );
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(128), "{text}");
    assert!(text.contains("Cause: remote repository or ref was not found"));
}

#[test]
fn git_hook_failure_is_classified() {
    let output = run_fake(
        "git",
        &["push"],
        "printf '%s\\n' 'error: pre-push hook failed' >&2; exit 1",
    );
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("Cause: Git hook failed"));
}

#[test]
fn git_global_options_are_parsed_before_subcommand() {
    for arguments in [
        vec!["-C", "/repo", "push"],
        vec!["-c", "credential.helper=", "fetch"],
        vec!["--git-dir=/repo/.git", "--work-tree", "/repo", "pull"],
    ] {
        let output = run_fake("git", &arguments, ":");
        let text = combined(&output);
        assert!(output.status.success(), "{text}");
        assert!(text.contains("Running: git "), "{text}");
        assert!(!text.contains("Running: git ["), "{text}");
    }
}

#[test]
fn git_help_and_version_are_not_misdetected() {
    for arguments in [vec!["--help", "push"], vec!["--version", "fetch"]] {
        let output = run_fake("git", &arguments, "printf '%s\\n' 'git help output'");
        let text = combined(&output);
        assert!(output.status.success(), "{text}");
        assert!(!text.contains("Running: git push"), "{text}");
        assert!(!text.contains("Running: git fetch"), "{text}");
        assert!(!text.contains("Result: no ref updates reported."), "{text}");
    }
}

#[test]
fn git_failure_is_classified_and_secrets_are_redacted() {
    let output = run_fake(
        "git",
        &["push"],
        "printf '%s\\n' 'Authorization: Bearer secret-value' 'token=first-secret token=second-secret password=third-secret password=fourth-secret' 'fatal: unable to access https://user:password@example.invalid/repo: Authentication failed' >&2; exit 128",
    );
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(128), "{text}");
    assert!(text.contains("Failure summary (git-transfer)"));
    assert!(text.contains("Cause: authentication or repository permission failed"));
    for secret in [
        "secret-value",
        "first-secret",
        "second-secret",
        "third-secret",
        "fourth-secret",
        "user:password",
    ] {
        assert!(!text.contains(secret), "{text}");
    }
    assert!(text.contains("[REDACTED]"));
}
