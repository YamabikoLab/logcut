#![cfg(target_os = "linux")]

mod common;

use common::TestDir;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_logcut")
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn help_flags_show_usage_options_and_profiles() {
    for flag in ["--help", "-h"] {
        let output = Command::new(binary()).arg(flag).output().unwrap();
        let text = combined(&output);

        assert!(output.status.success(), "{flag}: {text}");
        assert!(text.contains("Usage: logcut [OPTIONS] <command> [arguments...]"));
        assert!(text.contains("Options:"));
        assert!(text.contains("Profiles:"));
        for profile in [
            "auto",
            "jest",
            "vitest",
            "prettier",
            "eslint",
            "stylelint",
            "typescript",
            "phpunit",
            "phpstan",
            "php-lint",
            "phpcs",
            "phpcbf",
            "contract",
            "vite",
            "webpack",
            "composer",
            "playwright",
            "generic",
        ] {
            assert!(text.contains(profile), "missing profile {profile}: {text}");
        }
    }
}

#[test]
fn version_flags_show_package_version() {
    for flag in ["--version", "-V"] {
        let output = Command::new(binary()).arg(flag).output().unwrap();
        let text = combined(&output);

        assert!(output.status.success(), "{flag}: {text}");
        assert_eq!(text.trim(), format!("logcut {}", env!("CARGO_PKG_VERSION")));
    }
}

#[test]
fn help_flags_after_profile_show_help() {
    for flag in ["--help", "-h"] {
        let output = Command::new(binary())
            .args(["--profile=generic", flag])
            .output()
            .unwrap();
        let text = combined(&output);

        assert!(output.status.success(), "{flag}: {text}");
        assert!(text.contains("Usage: logcut [OPTIONS] <command> [arguments...]"));
        assert!(text.contains("Profiles:"));
        assert!(!text.contains("Running:"));
    }
}

#[test]
fn version_flags_after_profile_show_package_version() {
    for flag in ["--version", "-V"] {
        let output = Command::new(binary())
            .args(["--profile=generic", flag])
            .output()
            .unwrap();
        let text = combined(&output);

        assert!(output.status.success(), "{flag}: {text}");
        assert_eq!(text.trim(), format!("logcut {}", env!("CARGO_PKG_VERSION")));
    }
}

#[test]
fn command_help_argument_is_forwarded_to_the_child() {
    let root = TestDir::new("logcut-test", "child-help");
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
        .args(["sh", "-c", "test \"$1\" = \"--help\"", "_", "--help"])
        .output()
        .unwrap();
    let text = combined(&output);

    assert!(output.status.success(), "{text}");
    assert!(text.contains("Running: sh [4 args]"));
    assert!(text.contains("PASS ("));
    assert!(!text.contains("Profiles:"));
}

#[test]
fn unknown_profile_points_to_help() {
    let output = Command::new(binary())
        .arg("--profile=unknown")
        .arg("true")
        .output()
        .unwrap();
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(2));
    assert!(text.contains("Unknown profile: unknown"));
    assert!(text.contains("logcut --help"));
}

#[test]
fn missing_command_points_to_help() {
    let output = Command::new(binary()).output().unwrap();
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(2));
    assert!(text.contains("Usage: logcut [OPTIONS] <command> [arguments...]"));
    assert!(text.contains("logcut --help"));
}
