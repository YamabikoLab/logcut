#[path = "jest.rs"]
mod jest;

use crate::{playwright, SummarySettings};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Profile {
    Auto,
    Jest,
    Vitest,
    Prettier,
    Eslint,
    Typescript,
    Phpunit,
    Phpstan,
    PhpLint,
    Phpcs,
    Phpcbf,
    Contract,
    Vite,
    Webpack,
    Composer,
    Playwright,
    Generic,
}

impl Profile {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "auto" => Self::Auto,
            "jest" => Self::Jest,
            "vitest" => Self::Vitest,
            "prettier" => Self::Prettier,
            "eslint" => Self::Eslint,
            "typescript" => Self::Typescript,
            "phpunit" => Self::Phpunit,
            "phpstan" => Self::Phpstan,
            "php-lint" => Self::PhpLint,
            "phpcs" => Self::Phpcs,
            "phpcbf" => Self::Phpcbf,
            "contract" => Self::Contract,
            "vite" => Self::Vite,
            "webpack" => Self::Webpack,
            "composer" => Self::Composer,
            "playwright" => Self::Playwright,
            "generic" => Self::Generic,
            _ => return None,
        })
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Jest => "jest",
            Self::Vitest => "vitest",
            Self::Prettier => "prettier",
            Self::Eslint => "eslint",
            Self::Typescript => "typescript",
            Self::Phpunit => "phpunit",
            Self::Phpstan => "phpstan",
            Self::PhpLint => "php-lint",
            Self::Phpcs => "phpcs",
            Self::Phpcbf => "phpcbf",
            Self::Contract => "contract",
            Self::Vite => "vite",
            Self::Webpack => "webpack",
            Self::Composer => "composer",
            Self::Playwright => "playwright",
            Self::Generic => "generic",
        }
    }
}

pub(crate) fn detect_profile(output: &str) -> Profile {
    let lines: Vec<&str> = output.lines().collect();
    if playwright::detect(output) {
        Profile::Playwright
    } else if jest::detect(output) {
        Profile::Jest
    } else if output.contains("PHPCBF RESULT SUMMARY") {
        Profile::Phpcbf
    } else if output.contains("PHPCS REPORT SUMMARY")
        || output.contains("FILE: ") && output.contains("FOUND ") && output.contains("ERROR")
    {
        Profile::Phpcs
    } else if output.contains("webpack compiled with ")
        || output.contains("Module build failed")
        || output.contains("Module parse failed")
    {
        Profile::Webpack
    } else if lines
        .iter()
        .any(|line| line.starts_with(" FAIL  ") && line.contains(".test."))
    {
        Profile::Vitest
    } else if output.lines().any(contains_typescript_error) {
        Profile::Typescript
    } else if output.lines().any(is_eslint_error) {
        Profile::Eslint
    } else if output
        .lines()
        .any(|line| line.starts_with("[warn]") || line.contains("Code style issues found"))
    {
        Profile::Prettier
    } else if output.lines().any(|line| {
        line.starts_with("PHPUnit ")
            || line.starts_with("There was ")
            || line.starts_with("There were ")
            || line == "FAILURES!"
            || line == "ERRORS!"
    }) {
        Profile::Phpunit
    } else if output.lines().any(|line| {
        (line.trim_start().starts_with("[ERROR]") && line.contains("error"))
            || (line.contains("PHPStan") && line.to_ascii_lowercase().contains("error"))
    }) {
        Profile::Phpstan
    } else if output.contains("PHP Parse error:")
        || output.contains("PHP Fatal error:")
        || output.contains("Errors parsing ")
    {
        Profile::PhpLint
    } else if output
        .lines()
        .any(|line| line.starts_with("Contract check failed:"))
    {
        Profile::Contract
    } else if output.contains("error during build:")
        || output.contains("Build failed")
        || output.contains("RollupError")
        || output.contains("✗ Build failed")
    {
        Profile::Vite
    } else if output.contains("returned with error code")
        || output.contains("Your requirements could not be resolved")
        || output.contains("composer.json is invalid")
    {
        Profile::Composer
    } else {
        Profile::Generic
    }
}

pub(crate) fn successful_nonzero_exit(profile: Profile, status: i32, output: &str) -> bool {
    profile == Profile::Phpcbf
        && status == 1
        && output.contains("PHPCBF RESULT SUMMARY")
        && output.contains("A TOTAL OF ")
        && output.contains("ERRORS WERE FIXED")
        && !output.contains("FAILED TO FIX")
}

pub(crate) fn summarize(profile: Profile, output: &str, settings: &SummarySettings) -> Vec<String> {
    match profile {
        Profile::Jest => jest::summarize(output, settings.summary_lines),
        Profile::Vitest => summarize_vitest(output),
        Profile::Prettier => filter_lines(output, settings.summary_lines, |line| {
            line.starts_with("[warn]")
                || line.contains("Code style issues found")
                || line.contains("Forgot to run Prettier")
        }),
        Profile::Eslint => summarize_eslint(output, settings.max_errors),
        Profile::Typescript => summarize_typescript(output, settings.max_errors),
        Profile::Phpunit => summarize_phpunit(output, settings.summary_lines),
        Profile::Phpstan => summarize_phpstan(output, settings.summary_lines),
        Profile::PhpLint => filter_lines(output, settings.summary_lines, |line| {
            line.contains("PHP Parse error:")
                || line.contains("PHP Fatal error:")
                || line.contains("Errors parsing ")
        }),
        Profile::Phpcs | Profile::Phpcbf => summarize_php_codesniffer(output, settings.summary_lines),
        Profile::Contract => capture_from(
            output,
            settings.summary_lines,
            |line| line.starts_with("Contract check failed:"),
            "[additional contract failures omitted]",
        ),
        Profile::Vite => capture_from(
            output,
            settings.summary_lines,
            |line| {
                line.contains("error during build:")
                    || line.contains("Build failed")
                    || line.contains("RollupError")
                    || line.contains("✗ Build failed")
            },
            "[additional build output omitted]",
        ),
        Profile::Webpack => summarize_webpack(output, settings.summary_lines),
        Profile::Composer => summarize_composer(output, settings.summary_lines),
        Profile::Playwright => playwright::summarize(output, settings.summary_lines),
        Profile::Auto | Profile::Generic => tail_lines(output, settings.summary_lines),
    }
}

fn summarize_php_codesniffer(output: &str, maximum: usize) -> Vec<String> {
    let lines: Vec<&str> = output.lines().collect();
    let mut result = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.starts_with("FILE: ") {
            result.push((*line).to_string());
            for detail in lines.iter().skip(index + 1) {
                if detail.starts_with("FILE: ") || detail.contains("RESULT SUMMARY") {
                    break;
                }
                if !detail.trim().is_empty()
                    && (detail.contains("ERROR")
                        || detail.contains("WARNING")
                        || detail.contains("FOUND "))
                {
                    result.push((*detail).to_string());
                }
                if result.len() >= maximum {
                    return result;
                }
            }
        }
    }
    for line in lines.iter().filter(|line| {
        line.contains("A TOTAL OF ")
            || line.contains("FOUND ")
            || line.contains("ERRORS WERE FIXED")
            || line.contains("FAILED TO FIX")
    }) {
        if result.len() >= maximum {
            break;
        }
        if !result.iter().any(|value| value == line) {
            result.push((*line).to_string());
        }
    }
    result
}

fn summarize_webpack(output: &str, maximum: usize) -> Vec<String> {
    capture_from(
        output,
        maximum,
        |line| {
            line.starts_with("ERROR in ")
                || line.contains("Module build failed")
                || line.contains("Module parse failed")
                || line.contains("webpack compiled with ")
        },
        "[additional webpack output omitted]",
    )
}

fn summarize_vitest(output: &str) -> Vec<String> {
    let lines: Vec<&str> = output.lines().collect();
    let mut result = Vec::new();
    if let Some(start) = lines.iter().position(|line| line.starts_with(" FAIL  ")) {
        for line in &lines[start..] {
            if !result.is_empty() && line.starts_with('⎯') {
                break;
            }
            result.push((*line).to_string());
        }
    }
    for line in lines
        .iter()
        .filter(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("Test Files")
                || trimmed.starts_with("Tests")
                || trimmed.starts_with("Duration")
        })
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        result.push((*line).to_string());
    }
    result
}

fn summarize_eslint(output: &str, maximum: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut current_file: Option<&str> = None;
    let mut last_file: Option<&str> = None;
    let mut errors = 0usize;
    for line in output.lines() {
        let trimmed = line.trim();
        if !line.starts_with(char::is_whitespace)
            && [".cjs", ".js", ".jsx", ".mjs", ".ts", ".tsx"]
                .iter()
                .any(|suffix| trimmed.ends_with(suffix))
        {
            current_file = Some(line);
            continue;
        }
        if is_eslint_error(line) {
            if current_file != last_file {
                if let Some(file) = current_file {
                    result.push(file.to_string());
                }
                last_file = current_file;
            }
            result.push(line.to_string());
            errors += 1;
            if errors >= maximum {
                result.push("[additional ESLint errors omitted]".to_string());
                break;
            }
        }
    }
    if let Some(summary) = output
        .lines()
        .rev()
        .find(|line| line.starts_with('✖') && line.contains("problem"))
    {
        result.push(summary.to_string());
    }
    result
}

fn summarize_typescript(output: &str, maximum: usize) -> Vec<String> {
    let errors: Vec<&str> = output
        .lines()
        .filter(|line| contains_typescript_error(line))
        .collect();
    let mut result: Vec<String> = errors
        .iter()
        .take(maximum)
        .map(|line| (*line).to_string())
        .collect();
    if errors.len() > maximum {
        result.push(format!(
            "[{} additional TypeScript errors omitted]",
            errors.len() - maximum
        ));
    }
    result
}

fn summarize_phpunit(output: &str, maximum: usize) -> Vec<String> {
    let lines: Vec<&str> = output.lines().collect();
    let start = lines.iter().position(|line| {
        line.starts_with("There was ")
            || line.starts_with("There were ")
            || line == &"FAILURES!"
            || line == &"ERRORS!"
            || is_phpunit_test_heading(line)
    });
    let Some(start) = start else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for line in &lines[start..] {
        result.push((*line).to_string());
        if line.starts_with("Tests:") || result.len() >= maximum {
            break;
        }
    }
    if result.len() >= maximum && !result.last().is_some_and(|line| line.starts_with("Tests:")) {
        result.push("[additional PHPUnit output omitted]".to_string());
    }
    result
}

fn summarize_phpstan(output: &str, maximum: usize) -> Vec<String> {
    let lines: Vec<&str> = output.lines().collect();
    if let Some(index) = lines
        .iter()
        .rposition(|line| line.trim_start().starts_with("[ERROR]"))
    {
        let start = index.saturating_add(1).saturating_sub(maximum);
        return lines[start..=index]
            .iter()
            .map(|line| (*line).to_string())
            .collect();
    }
    lines
        .iter()
        .filter(|line| {
            line.contains("PHPStan")
                || line.to_ascii_lowercase().contains("found ")
                    && line.to_ascii_lowercase().contains(" error")
        })
        .rev()
        .take(maximum)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|line| (*line).to_string())
        .collect()
}

fn summarize_composer(output: &str, maximum: usize) -> Vec<String> {
    let lines: Vec<&str> = output.lines().collect();
    let mut selected = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let lower = line.to_ascii_lowercase();
        if line.contains("returned with error code")
            || line.contains("Your requirements could not be resolved")
            || line.contains("composer.json is invalid")
            || lower.contains("does not match the expected json schema")
            || line.contains("[RuntimeException]")
            || line.contains("[InvalidArgumentException]")
        {
            let start = index.saturating_sub(2);
            let end = usize::min(lines.len(), index + 5);
            selected.extend(lines[start..end].iter().map(|value| (*value).to_string()));
        }
    }
    selected
        .into_iter()
        .rev()
        .take(maximum)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn capture_from<F>(output: &str, maximum: usize, starts: F, omitted: &str) -> Vec<String>
where
    F: Fn(&str) -> bool,
{
    let lines: Vec<&str> = output.lines().collect();
    let Some(start) = lines.iter().position(|line| starts(line)) else {
        return Vec::new();
    };
    let mut result: Vec<String> = lines[start..]
        .iter()
        .take(maximum)
        .map(|line| (*line).to_string())
        .collect();
    if lines.len() - start > maximum {
        result.push(omitted.to_string());
    }
    result
}

fn filter_lines<F>(output: &str, maximum: usize, predicate: F) -> Vec<String>
where
    F: Fn(&str) -> bool,
{
    output
        .lines()
        .filter(|line| predicate(line))
        .take(maximum)
        .map(str::to_string)
        .collect()
}

fn tail_lines(output: &str, maximum: usize) -> Vec<String> {
    let lines: Vec<&str> = output.lines().collect();
    lines[lines.len().saturating_sub(maximum)..]
        .iter()
        .map(|line| (*line).to_string())
        .collect()
}

fn contains_typescript_error(line: &str) -> bool {
    line.find("error TS").is_some_and(|index| {
        let rest = &line[index + 8..];
        let digits = rest
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .count();
        digits > 0 && rest.as_bytes().get(digits) == Some(&b':')
    })
}

fn is_eslint_error(line: &str) -> bool {
    let trimmed = line.trim_start();
    let mut parts = trimmed.split_whitespace();
    let Some(position) = parts.next() else {
        return false;
    };
    position
        .split_once(':')
        .is_some_and(|(line_number, column)| {
            !line_number.is_empty()
                && !column.is_empty()
                && line_number
                    .chars()
                    .all(|character| character.is_ascii_digit())
                && column.chars().all(|character| character.is_ascii_digit())
        })
        && parts.next() == Some("error")
}

fn is_phpunit_test_heading(line: &str) -> bool {
    let Some((number, rest)) = line.split_once(") ") else {
        return false;
    };
    number.chars().all(|character| character.is_ascii_digit()) && rest.contains("::")
}
