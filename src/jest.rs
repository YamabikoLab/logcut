pub(crate) fn detect(output: &str) -> bool {
    output.lines().any(is_failure_file)
        && output.lines().any(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("Test Suites:")
                || trimmed.starts_with("Snapshots:")
                || trimmed.starts_with("Ran all test suites")
        })
}

pub(crate) fn summarize(output: &str, maximum: usize) -> Vec<String> {
    let lines: Vec<&str> = output.lines().collect();
    let mut result = Vec::new();

    for line in lines.iter().copied().filter(|line| is_failure_file(line)) {
        push_unique(&mut result, line.to_string(), maximum);
    }

    for line in lines.iter().copied().filter(|line| is_failed_test(line)) {
        push_unique(&mut result, line.to_string(), maximum);
    }

    for line in lines.iter().copied().filter(|line| is_relevant_error(line)) {
        push_unique(&mut result, line.to_string(), maximum);
    }

    let summary_lines: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| is_final_summary(line))
        .collect();
    let reserved = summary_lines.len().min(maximum);
    if result.len() > maximum.saturating_sub(reserved) {
        result.truncate(maximum.saturating_sub(reserved));
    }
    for line in summary_lines {
        push_unique(&mut result, line.to_string(), maximum);
    }

    result
}

fn push_unique(result: &mut Vec<String>, line: String, maximum: usize) {
    if result.len() < maximum && !result.contains(&line) {
        result.push(line);
    }
}

fn is_failure_file(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("FAIL ") && !trimmed.starts_with("FAIL  ")
}

fn is_failed_test(line: &str) -> bool {
    line.trim_start().starts_with("● ")
}

fn is_relevant_error(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("Error:")
        || trimmed.starts_with("AssertionError:")
        || trimmed.starts_with("Expected:")
        || trimmed.starts_with("Received:")
        || trimmed.starts_with("Matcher error:")
}

fn is_final_summary(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("Test Suites:")
        || trimmed.starts_with("Tests:")
        || trimmed.starts_with("Snapshots:")
        || trimmed.starts_with("Time:")
        || trimmed.starts_with("Ran all test suites")
}
