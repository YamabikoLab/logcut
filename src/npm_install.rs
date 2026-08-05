const ERROR_CODES: [&str; 11] = [
    "ERESOLVE",
    "EUSAGE",
    "ETARGET",
    "E404",
    "E401",
    "E403",
    "EACCES",
    "ENOSPC",
    "ECONNRESET",
    "ENETUNREACH",
    "CERT_HAS_EXPIRED",
];

pub(crate) fn summarize(output: &str, maximum: usize, max_errors: usize) -> Vec<String> {
    let code = find_error_code(output);
    let mut result = Vec::new();

    if let Some(code) = code {
        push_unique(&mut result, format!("Code: {code}"), maximum);
    }

    if let Some(cause) = classify(code, output) {
        push_unique(&mut result, format!("Cause: {cause}"), maximum);
    }

    let detail_limit = max_errors.min(maximum.saturating_sub(result.len()));
    let mut details = Vec::new();
    let mut context_remaining = 0usize;

    for line in output.lines() {
        if is_npm_warning(line) {
            continue;
        }

        let detail = strip_npm_prefix(line);
        if detail.is_empty() || is_noise(detail) {
            continue;
        }

        if is_context_marker(detail) {
            push_unique(&mut details, detail.to_string(), detail_limit);
            context_remaining = 3;
            continue;
        }

        if context_remaining > 0 && is_context_detail(detail) {
            push_unique(&mut details, detail.to_string(), detail_limit);
            context_remaining -= 1;
            continue;
        }

        if is_operational_error(detail) {
            push_unique(&mut details, detail.to_string(), detail_limit);
        }
    }

    for detail in details {
        push_unique(&mut result, detail, maximum);
    }

    result
}

pub(crate) fn summarize_success(output: &str, maximum: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut peer_warnings = 0usize;
    let mut deprecated_warnings = 0usize;
    let mut vulnerability = None;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let lower = trimmed.to_ascii_lowercase();
        if is_package_change_summary(&lower) {
            push_unique(&mut result, trimmed.to_string(), maximum);
        }
        if is_peer_dependency_warning_start(&lower) {
            peer_warnings += 1;
        }
        if lower.contains("npm warn deprecated") {
            deprecated_warnings += 1;
        }
        if lower.contains("vulnerabilit")
            && (lower.starts_with("found ")
                || lower
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_digit()))
        {
            vulnerability = Some(trimmed.to_string());
        }
    }

    if peer_warnings > 0 {
        push_unique(
            &mut result,
            format!("Peer dependency warnings: {peer_warnings}"),
            maximum,
        );
    }
    if deprecated_warnings > 0 {
        push_unique(
            &mut result,
            format!("Deprecated warnings: {deprecated_warnings}"),
            maximum,
        );
    }
    if let Some(vulnerability) = vulnerability {
        push_unique(&mut result, vulnerability, maximum);
    }

    if result.is_empty() && maximum > 0 {
        result.push("npm dependency installation completed successfully.".to_string());
    }

    result
}

fn find_error_code(output: &str) -> Option<&'static str> {
    for line in output.lines() {
        let Some(detail) = strip_npm_error_prefix(line) else {
            continue;
        };
        let mut tokens =
            detail.split(|character: char| !character.is_ascii_alphanumeric() && character != '_');

        if tokens
            .next()
            .is_some_and(|token| token.eq_ignore_ascii_case("code"))
        {
            for token in tokens {
                if let Some(code) = ERROR_CODES.into_iter().find(|code| token == *code) {
                    return Some(code);
                }
            }
        }
    }

    for line in output.lines() {
        if is_npm_warning(line) {
            continue;
        }

        if let Some(code) = find_known_code(strip_npm_prefix(line)) {
            return Some(code);
        }
    }

    None
}

fn find_known_code(detail: &str) -> Option<&'static str> {
    detail
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .find_map(|token| ERROR_CODES.into_iter().find(|code| token == *code))
}

fn classify(code: Option<&str>, output: &str) -> Option<&'static str> {
    match code {
        Some("ERESOLVE") => Some("dependency tree could not be resolved"),
        Some("EUSAGE") => Some("package.json and lock file are not in sync"),
        Some("ETARGET") => Some("requested package version does not exist"),
        Some("E404") => Some("package or registry resource was not found"),
        Some("E401") => Some("registry authentication failed"),
        Some("E403") => Some("registry access was forbidden"),
        Some("EACCES") => Some("filesystem permission was denied"),
        Some("ENOSPC") => Some("disk space was exhausted"),
        Some("ECONNRESET") => Some("network connection was reset"),
        Some("ENETUNREACH") => Some("network was unreachable"),
        Some("CERT_HAS_EXPIRED") => Some("TLS certificate has expired"),
        _ => classify_from_output(output),
    }
}

fn classify_from_output(output: &str) -> Option<&'static str> {
    let lower = output
        .lines()
        .filter(|line| !is_npm_warning(line))
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();

    if lower.contains("could not resolve dependency")
        || lower.contains("conflicting peer dependency")
    {
        Some("dependency tree could not be resolved")
    } else if lower.contains("npm ci can only install packages when")
        || lower.contains("package.json and package-lock.json") && lower.contains("in sync")
    {
        Some("package.json and lock file are not in sync")
    } else if lower.contains("no matching version found") {
        Some("requested package version does not exist")
    } else if lower.contains("unable to authenticate") || lower.contains("authentication token") {
        Some("registry authentication failed")
    } else if lower.contains("permission denied") {
        Some("filesystem permission was denied")
    } else if lower.contains("no space left on device") {
        Some("disk space was exhausted")
    } else if lower.contains("network is unreachable") {
        Some("network was unreachable")
    } else if lower.contains("certificate has expired") || lower.contains("cert_has_expired") {
        Some("TLS certificate has expired")
    } else if lower.contains("integrity checksum failed") || lower.contains("eintegrity") {
        Some("package integrity verification failed")
    } else {
        None
    }
}

fn strip_npm_error_prefix(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    for prefix in ["npm ERR!", "npm error"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return Some(rest.trim_start_matches(['!', ':']).trim());
        }
    }
    None
}

fn strip_npm_prefix(line: &str) -> &str {
    if let Some(detail) = strip_npm_error_prefix(line) {
        return detail;
    }

    let trimmed = line.trim();
    for prefix in ["npm WARN", "npm warn"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.trim_start_matches(['!', ':']).trim();
        }
    }
    trimmed
}

fn is_npm_warning(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("npm WARN") || trimmed.starts_with("npm warn")
}

fn is_peer_dependency_warning_start(lower: &str) -> bool {
    lower.starts_with("npm warn eresolve")
        && (lower.contains("peer dependency") || lower.contains("peer dep"))
}

fn is_context_marker(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    lower.starts_with("while resolving:")
        || lower.starts_with("found:")
        || lower.starts_with("could not resolve dependency:")
        || lower.starts_with("conflicting peer dependency:")
        || lower.starts_with("invalid:")
        || lower.starts_with("missing:")
        || lower.contains("npm ci can only install packages when")
        || lower.contains("package.json and package-lock.json")
        || lower.contains("lock file's")
        || lower.contains("no matching version found")
        || lower.contains("is not in this registry")
}

fn is_context_detail(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    detail.contains('@')
        || lower.starts_with("peer ")
        || lower.starts_with("node_modules/")
        || lower.starts_with("from ")
        || lower.starts_with("required:")
        || lower.starts_with("actual:")
        || lower.starts_with("missing:")
        || lower.starts_with("invalid:")
}

fn is_operational_error(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    lower.starts_with("code ")
        || lower.contains("unable to authenticate")
        || lower.contains("authentication token")
        || lower.contains("permission denied")
        || lower.contains("access is denied")
        || lower.contains("no space left on device")
        || lower.contains("network is unreachable")
        || lower.contains("connection reset")
        || lower.contains("certificate has expired")
        || lower.contains("cert_has_expired")
        || lower.contains("integrity checksum failed")
        || lower.contains("eintegrity")
        || lower.contains("not in this registry")
}

fn is_noise(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    lower.starts_with("a complete log of this run can be found in:")
        || lower.starts_with("for a full report see:")
        || lower.starts_with("fix the upstream dependency conflict")
        || lower.starts_with("retry this command with --force")
}

fn is_package_change_summary(lower: &str) -> bool {
    !lower.contains("npm warn")
        && lower.contains("package")
        && (lower.starts_with("added ")
            || lower.starts_with("removed ")
            || lower.starts_with("changed ")
            || lower.contains(", added ")
            || lower.contains(", removed ")
            || lower.contains(", changed "))
}

fn push_unique(result: &mut Vec<String>, line: String, maximum: usize) {
    if result.len() < maximum && !result.contains(&line) {
        result.push(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_dependency_resolution_failure() {
        let output = "npm error code ERESOLVE\nnpm error While resolving: example@1.0.0\nnpm error Found: react@18.3.1\nnpm error node_modules/react\nnpm error Could not resolve dependency:\nnpm error peer react@\"^17\" from legacy-plugin@2.0.0\nnpm error Conflicting peer dependency: react@17.0.2\n";
        let summary = summarize(output, 20, 20);

        assert_eq!(summary[0], "Code: ERESOLVE");
        assert_eq!(summary[1], "Cause: dependency tree could not be resolved");
        assert!(summary
            .iter()
            .any(|line| line == "While resolving: example@1.0.0"));
        assert!(summary.iter().any(|line| line == "Found: react@18.3.1"));
        assert!(summary
            .iter()
            .any(|line| line == "peer react@\"^17\" from legacy-plugin@2.0.0"));
    }

    #[test]
    fn prioritizes_explicit_error_after_peer_dependency_warning_block() {
        let output = "npm warn ERESOLVE overriding peer dependency\nnpm warn While resolving: example@1.0.0\nnpm warn Found: react@18.3.1\nnpm warn Conflicting peer dependency: react@17.0.2\nnpm error code EACCES\nnpm error syscall mkdir\nnpm error path /usr/local/lib/node_modules/example\nnpm error Error: EACCES: permission denied, mkdir '/usr/local/lib/node_modules/example'\n";
        let summary = summarize(output, 20, 20);

        assert_eq!(summary[0], "Code: EACCES");
        assert_eq!(summary[1], "Cause: filesystem permission was denied");
        assert!(summary.iter().any(|line| line == "code EACCES"));
        assert!(summary.iter().any(|line| line.contains("permission denied")));
        assert!(!summary.iter().any(|line| line.contains("ERESOLVE")));
        assert!(!summary
            .iter()
            .any(|line| line.contains("Conflicting peer dependency")));
    }

    #[test]
    fn summarizes_lock_file_mismatch() {
        let output = "npm error code EUSAGE\nnpm error `npm ci` can only install packages when your package.json and package-lock.json are in sync.\nnpm error Invalid: lock file's eslint@8.0.0 does not satisfy eslint@9.0.0\n";
        let summary = summarize(output, 20, 20);

        assert_eq!(summary[0], "Code: EUSAGE");
        assert_eq!(
            summary[1],
            "Cause: package.json and lock file are not in sync"
        );
        assert!(summary
            .iter()
            .any(|line| line.contains("package-lock.json")));
        assert!(summary.iter().any(|line| line.contains("eslint@8.0.0")));
    }

    #[test]
    fn summarizes_success_without_routine_noise() {
        let output = "npm warn deprecated old-package@1.0.0: no longer supported\nnpm warn ERESOLVE overriding peer dependency\nnpm warn Found: react@18.3.1\nnpm warn node_modules/react\nnpm warn Conflicting peer dependency: react@17.0.2\nadded 12 packages, removed 1 package, and changed 2 packages in 3s\n3 vulnerabilities (1 moderate, 2 high)\n";
        let summary = summarize_success(output, 20);

        assert!(summary[0].contains("added 12 packages"));
        assert!(summary
            .iter()
            .any(|line| line == "Peer dependency warnings: 1"));
        assert!(summary.iter().any(|line| line == "Deprecated warnings: 1"));
        assert!(summary
            .iter()
            .any(|line| line == "3 vulnerabilities (1 moderate, 2 high)"));
    }
}
