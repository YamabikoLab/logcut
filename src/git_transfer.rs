pub(crate) fn detect(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("failed to push some refs")
        || lower.contains("non-fast-forward")
        || lower.contains("has no upstream branch")
        || lower.contains("no tracking information")
        || lower.contains("repository not found")
        || lower.contains("does not appear to be a git repository")
        || lower.contains("host key verification failed")
        || lower.contains("automatic merge failed")
        || output.lines().any(|line| {
            let trimmed = line.trim();
            trimmed == "Everything up-to-date"
                || trimmed == "Already up to date."
                || trimmed == "Already up-to-date."
                || trimmed == "Fast-forward"
                || trimmed.starts_with("Updating ")
                || is_ref_update(trimmed)
        })
}

pub(crate) fn summarize(output: &str, maximum: usize, max_errors: usize) -> Vec<String> {
    let mut result = Vec::new();

    if let Some(cause) = classify(output) {
        push_unique(&mut result, format!("Cause: {cause}"), maximum);
    }

    if let Some(remote) = output.lines().find_map(remote_line) {
        push_unique(&mut result, remote, maximum);
    }

    let relevant: Vec<String> = output
        .lines()
        .filter(|line| is_relevant_failure(line))
        .map(|line| line.trim().to_string())
        .collect();
    let limit = max_errors.min(maximum.saturating_sub(result.len()));
    let start = relevant.len().saturating_sub(limit);
    for line in &relevant[start..] {
        push_unique(&mut result, line.clone(), maximum);
    }

    result
}

pub(crate) fn summarize_success(output: &str, maximum: usize) -> Vec<String> {
    let mut result = Vec::new();

    for line in output.lines() {
        if let Some(remote) = remote_line(line) {
            push_unique(&mut result, remote, maximum);
            continue;
        }

        let trimmed = line.trim();
        if trimmed == "Everything up-to-date"
            || trimmed == "Already up to date."
            || trimmed == "Already up-to-date."
            || trimmed == "Fast-forward"
            || trimmed.starts_with("Updating ")
            || is_ref_update(trimmed)
        {
            push_unique(&mut result, trimmed.to_string(), maximum);
        }
    }

    if result.is_empty() && maximum > 0 {
        result.push("Result: no ref updates reported.".to_string());
    }

    result
}

fn classify(output: &str) -> Option<&'static str> {
    let lower = output.to_ascii_lowercase();

    if lower.contains("non-fast-forward")
        || lower.contains("fetch first")
        || lower.contains("updates were rejected because the tip")
    {
        Some("non-fast-forward update rejected")
    } else if lower.contains("has no upstream branch")
        || lower.contains("no tracking information")
        || lower.contains("set-upstream")
    {
        Some("upstream branch is not configured")
    } else if lower.contains("conflict (")
        || lower.contains("automatic merge failed")
        || lower.contains("not possible to fast-forward")
    {
        Some("merge conflict")
    } else if lower.contains("host key verification failed")
        || lower.contains("remote host identification has changed")
        || lower.contains("could not resolve hostname")
        || lower.contains("connection closed by") && lower.contains("port 22")
    {
        Some("SSH connection or host-key verification failed")
    } else if lower.contains("authentication failed")
        || lower.contains("permission denied (publickey")
        || lower.contains("could not read username")
        || lower.contains("access denied")
        || lower.contains("permission to ") && lower.contains(" denied")
    {
        Some("authentication or repository permission failed")
    } else if lower.contains("repository not found")
        || lower.contains("does not appear to be a git repository")
        || lower.contains("couldn't find remote ref")
    {
        Some("remote repository or ref was not found")
    } else if lower.contains("protected branch")
        || lower.contains("branch protection")
        || lower.contains("gh006")
        || lower.contains("protected branch hook declined")
    {
        Some("protected branch or repository policy rejected the update")
    } else if lower.contains("pre-push hook")
        || lower.contains("hook declined")
        || lower.contains("hook failed")
    {
        Some("Git hook failed")
    } else if lower.contains("could not resolve host")
        || lower.contains("failed to connect")
        || lower.contains("connection timed out")
        || lower.contains("network is unreachable")
        || lower.contains("ssl certificate problem")
        || lower.contains("tls") && lower.contains("error")
    {
        Some("network, DNS, or TLS connection failed")
    } else {
        None
    }
}

fn remote_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let remote = trimmed
        .strip_prefix("To ")
        .or_else(|| trimmed.strip_prefix("From "))?;
    if remote.is_empty() {
        None
    } else {
        Some(format!("Remote: {remote}"))
    }
}

fn is_ref_update(line: &str) -> bool {
    line.contains(" -> ")
        && (line.contains("..")
            || line.contains("[new branch]")
            || line.contains("[new tag]")
            || line.contains("[up to date]")
            || line.contains("[rejected]")
            || line.starts_with('*')
            || line.starts_with('+')
            || line.starts_with('-')
            || line.starts_with('='))
}

fn is_relevant_failure(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    trimmed.starts_with("fatal:")
        || trimmed.starts_with("error:")
        || trimmed.starts_with("CONFLICT")
        || trimmed.starts_with("hint:")
        || trimmed.contains("[rejected]")
        || lower.contains("non-fast-forward")
        || lower.contains("has no upstream branch")
        || lower.contains("no tracking information")
        || lower.contains("authentication failed")
        || lower.contains("permission denied")
        || lower.contains("host key verification failed")
        || lower.contains("remote host identification has changed")
        || lower.contains("repository not found")
        || lower.contains("does not appear to be a git repository")
        || lower.contains("protected branch")
        || lower.contains("branch protection")
        || lower.contains("hook declined")
        || lower.contains("hook failed")
        || lower.contains("could not resolve host")
        || lower.contains("could not resolve hostname")
        || lower.contains("failed to connect")
        || lower.contains("connection timed out")
        || lower.contains("ssl certificate problem")
        || lower.contains("automatic merge failed")
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
    fn summarizes_push_success() {
        let output = "To github.com:YamabikoLab/logcut.git\n   1111111..2222222  main -> main\n";
        let summary = summarize_success(output, 10);
        assert_eq!(summary[0], "Remote: github.com:YamabikoLab/logcut.git");
        assert!(summary[1].contains("1111111..2222222"));
    }

    #[test]
    fn classifies_common_failures() {
        assert_eq!(
            summarize(
                "fatal: The current branch x has no upstream branch.\n",
                10,
                10
            )[0],
            "Cause: upstream branch is not configured"
        );
        assert_eq!(
            summarize(
                "CONFLICT (content): Merge conflict\nAutomatic merge failed\n",
                10,
                10
            )[0],
            "Cause: merge conflict"
        );
        assert_eq!(
            summarize("Host key verification failed.\n", 10, 10)[0],
            "Cause: SSH connection or host-key verification failed"
        );
    }
}
