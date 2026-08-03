#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

#[cfg(not(target_os = "linux"))]
compile_error!("logcut currently supports Linux only");

mod docker_build;
mod git_transfer;
mod logging;
mod phpcbf;
mod playwright;
mod process;
mod stylelint;
mod summary;

use logging::{normalize_output, prepare_log_file, prune_logs, read_log};
use phpcbf::successful_nonzero_exit;
use process::{run_direct, run_suppressed};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use summary::{
    command_profile, detect_profile, recognized_command_label, summarize, summarize_success, Profile,
};

const USAGE: &str = "Usage: logcut [OPTIONS] <command> [arguments...]";

pub(crate) struct Settings {
    pub(crate) profile: Profile,
    pub(crate) summary_lines: usize,
    pub(crate) max_errors: usize,
    pub(crate) max_log_files: usize,
    pub(crate) max_log_age_days: u64,
    pub(crate) log_directory: PathBuf,
}

pub(crate) struct SummarySettings {
    pub(crate) summary_lines: usize,
    pub(crate) max_errors: usize,
}

enum CommandLine {
    Run {
        arguments: Vec<OsString>,
        profile: Profile,
    },
    Exit(i32),
}

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("logcut: {error}");
            1
        }
    };
    std::process::exit(code);
}

fn run() -> io::Result<i32> {
    let original_umask = set_private_umask();

    let (arguments, profile) = match read_command_line() {
        CommandLine::Run { arguments, profile } => (arguments, profile),
        CommandLine::Exit(code) => return Ok(code),
    };
    let settings = settings_from_environment(profile);

    run_command(&arguments, &settings, original_umask)
}

fn set_private_umask() -> libc::mode_t {
    unsafe { libc::umask(0o077) }
}

fn read_command_line() -> CommandLine {
    let mut arguments: Vec<OsString> = env::args_os().skip(1).collect();
    let mut profile_value = env::var("LOGCUT_PROFILE").unwrap_or_else(|_| "auto".to_string());

    while let Some(first) = arguments.first().and_then(|value| value.to_str()) {
        match first {
            "--help" | "-h" => {
                print_help();
                return CommandLine::Exit(0);
            }
            "--version" | "-V" => {
                println!("logcut {}", env!("CARGO_PKG_VERSION"));
                return CommandLine::Exit(0);
            }
            _ => {}
        }

        let Some(value) = first.strip_prefix("--profile=") else {
            break;
        };
        profile_value = value.to_string();
        arguments.remove(0);
    }

    let Some(profile) = Profile::parse(&profile_value) else {
        eprintln!("Unknown profile: {profile_value}");
        eprintln!("Run 'logcut --help' to see the available profiles.");
        return CommandLine::Exit(2);
    };

    if arguments.is_empty() {
        eprintln!("{USAGE}");
        eprintln!("Run 'logcut --help' for more information.");
        return CommandLine::Exit(2);
    }

    CommandLine::Run { arguments, profile }
}

fn print_help() {
    println!(
        "logcut - run commands quietly and show concise failure summaries\n\n\
{USAGE}\n\n\
Options:\n\
  --profile=PROFILE  Select the failure-summary profile (default: auto)\n\
  -h, --help         Print help\n\
  -V, --version      Print version\n\n\
Profiles:\n\
  auto          Detect the profile from command output\n\
  jest          Summarize Jest test failures\n\
  vitest        Summarize Vitest test failures\n\
  prettier      Summarize Prettier formatting failures\n\
  eslint        Summarize ESLint errors\n\
  stylelint     Summarize Stylelint errors\n\
  typescript    Summarize TypeScript compiler errors\n\
  phpunit       Summarize PHPUnit test failures\n\
  phpstan       Summarize PHPStan analysis errors\n\
  php-lint      Summarize PHP syntax errors\n\
  phpcs         Summarize PHP_CodeSniffer violations\n\
  phpcbf        Summarize PHP Code Beautifier and Fixer results\n\
  contract      Summarize contract-check failures\n\
  vite          Summarize Vite build failures\n\
  webpack       Summarize webpack build failures\n\
  composer      Summarize Composer failures\n\
  playwright    Summarize Playwright test failures\n\
  docker-build  Summarize Docker build and Docker Compose build results\n\
  git-transfer  Summarize Git push, pull, and fetch results\n\
  generic       Show the tail of the command output\n\n\
logcut options are recognized only before the command. For example,\n\
'logcut --help' prints this help, while 'logcut npm --help' runs npm --help."
    );
}

fn settings_from_environment(profile: Profile) -> Settings {
    Settings {
        profile,
        summary_lines: positive_setting(
            "LOGCUT_SUMMARY_LINES",
            env::var("LOGCUT_SUMMARY_LINES")
                .ok()
                .or_else(|| env::var("LOGCUT_TAIL_LINES").ok()),
            40,
            200,
        ),
        max_errors: positive_setting(
            "LOGCUT_MAX_ERRORS",
            env::var("LOGCUT_MAX_ERRORS").ok(),
            20,
            100,
        ),
        max_log_files: positive_setting(
            "LOGCUT_LOG_MAX_FILES",
            env::var("LOGCUT_LOG_MAX_FILES").ok(),
            10,
            100,
        ),
        max_log_age_days: positive_setting(
            "LOGCUT_LOG_MAX_AGE_DAYS",
            env::var("LOGCUT_LOG_MAX_AGE_DAYS").ok(),
            7,
            30,
        ) as u64,
        log_directory: env::var_os("LOGCUT_LOG_DIRECTORY")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("/tmp/logcut-{}", current_user_id()))),
    }
}

fn current_user_id() -> libc::uid_t {
    unsafe { libc::getuid() }
}

fn run_command(
    arguments: &[OsString],
    settings: &Settings,
    original_umask: libc::mode_t,
) -> io::Result<i32> {
    let label = command_label(arguments);
    let start = Instant::now();
    println!("Running: {label}");
    let _ = io::stdout().flush();

    let log_path = match prepare_log_file(settings) {
        Ok(path) => path,
        Err(_) => {
            eprintln!(
                "logcut: secure logging is unavailable; running command without output suppression"
            );
            return run_direct(arguments, original_umask);
        }
    };

    let status = match run_suppressed(arguments, &log_path, original_umask) {
        Ok(status) => status,
        Err(error) => {
            let _ = fs::remove_file(&log_path);
            eprintln!(
                "logcut: command setup failed ({error}); running command without output suppression"
            );
            return run_direct(arguments, original_umask);
        }
    };

    handle_command_result(
        status,
        start.elapsed().as_secs(),
        &label,
        arguments,
        &log_path,
        settings,
    )
}

fn handle_command_result(
    status: i32,
    elapsed: u64,
    label: &str,
    arguments: &[OsString],
    log_path: &Path,
    settings: &Settings,
) -> io::Result<i32> {
    if status == 0 {
        print_success(elapsed, label, arguments, log_path, settings);
        let _ = fs::remove_file(log_path);
        return Ok(0);
    }

    let raw = match read_log(log_path) {
        Ok(raw) => raw,
        Err(error) => {
            eprintln!("FAIL ({elapsed}s, exit {status}): {label}");
            eprintln!("logcut: failure summary could not be generated: {error}");
            eprintln!("Full log: {}", log_path.display());
            return Ok(status);
        }
    };
    let clean = normalize_output(raw);
    let selected = selected_profile(settings.profile, arguments, &clean);
    if successful_nonzero_exit(selected, status, &clean) {
        println!("PASS ({elapsed}s): {label}");
        let _ = fs::remove_file(log_path);
        return Ok(0);
    }

    let summary_settings = SummarySettings {
        summary_lines: settings.summary_lines,
        max_errors: settings.max_errors,
    };
    let mut summary = summarize(selected, &clean, &summary_settings);
    if !summary.iter().any(|line| !line.trim().is_empty()) {
        summary = summarize(Profile::Generic, &clean, &summary_settings);
    }
    limit_summary(&mut summary, settings.summary_lines);

    eprintln!("FAIL ({elapsed}s, exit {status}): {label}");
    eprintln!("\n----- Failure summary ({}) -----", selected.as_str());
    for line in summary {
        eprintln!("{line}");
    }
    eprintln!("\nFull log: {}", log_path.display());

    prune_logs(
        &settings.log_directory,
        settings.max_log_age_days,
        settings.max_log_files,
    );
    Ok(status)
}

fn print_success(
    elapsed: u64,
    label: &str,
    arguments: &[OsString],
    log_path: &Path,
    settings: &Settings,
) {
    let selected = if settings.profile == Profile::Auto {
        command_profile(arguments)
    } else {
        Some(settings.profile)
    };

    println!("PASS ({elapsed}s): {label}");

    let Some(selected) = selected else {
        return;
    };
    if !matches!(selected, Profile::DockerBuild | Profile::GitTransfer) {
        return;
    }
    let Ok(raw) = read_log(log_path) else {
        return;
    };
    let clean = normalize_output(raw);
    let summary_settings = SummarySettings {
        summary_lines: settings.summary_lines,
        max_errors: settings.max_errors,
    };
    let mut summary = summarize_success(selected, &clean, &summary_settings);
    limit_summary(&mut summary, settings.summary_lines);
    for line in summary {
        println!("{line}");
    }
}

fn selected_profile(configured: Profile, arguments: &[OsString], output: &str) -> Profile {
    if configured == Profile::Auto {
        command_profile(arguments).unwrap_or_else(|| detect_profile(output))
    } else {
        configured
    }
}

fn limit_summary(summary: &mut Vec<String>, maximum: usize) {
    if summary.len() <= maximum {
        return;
    }

    summary.truncate(maximum.saturating_sub(1));
    summary.push("[additional summary lines omitted]".to_string());
}

fn positive_setting(name: &str, value: Option<String>, fallback: usize, maximum: usize) -> usize {
    let Some(value) = value else {
        return fallback;
    };
    match value.parse::<usize>() {
        Ok(number) if number > 0 && number <= maximum => number,
        _ => {
            eprintln!("logcut: invalid {name}={value:?}; using {fallback}");
            fallback
        }
    }
}

fn command_label(arguments: &[OsString]) -> String {
    if let Some(label) = recognized_command_label(arguments) {
        return label.to_string();
    }

    let name = Path::new(&arguments[0])
        .file_name()
        .unwrap_or_else(|| OsStr::new(""))
        .to_string_lossy();
    let sanitized: String = name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || "_.+-".contains(*character))
        .collect();
    let command = if sanitized.is_empty() {
        "<command>".to_string()
    } else {
        sanitized
    };
    let argument_count = arguments.len() - 1;
    if argument_count == 0 {
        command
    } else {
        format!("{command} [{argument_count} args]")
    }
}
