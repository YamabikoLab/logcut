const SCRIPT_EXTENSIONS: [&str; 6] = ["cjs", "js", "jsx", "mjs", "ts", "tsx"];

pub(crate) fn detect(output: &str) -> bool {
    output.lines().any(is_failure_heading)
        && (output.contains("Call log:")
            || output.contains("attachment #")
            || output.contains("trace.zip")
            || output.contains("test-results/"))
}

pub(crate) fn summarize(output: &str, maximum: usize) -> Vec<String> {
    let lines: Vec<&str> = output.lines().collect();
    let Some(heading_index) = lines.iter().position(|line| is_failure_heading(line)) else {
        return Vec::new();
    };

    let heading = lines[heading_index].to_string();
    let mut error = None;
    let mut fallback = None;
    let mut call_log = None;
    let mut call_detail = None;
    let mut attachment = None;
    let mut attachment_path = None;
    let mut result_lines = Vec::new();
    let mut in_call_log = false;

    for line in &lines[heading_index + 1..] {
        let trimmed = line.trim();
        if is_result_summary(trimmed) {
            result_lines.push(normalize_result_summary(trimmed));
            continue;
        }
        if error.is_none() && is_error_line(trimmed) {
            error = Some((*line).to_string());
        }
        if fallback.is_none()
            && !trimmed.is_empty()
            && !trimmed.starts_with("Call log:")
            && !trimmed.starts_with("attachment #")
            && !trimmed.chars().all(|character| matches!(character, '-' | '='))
        {
            fallback = Some((*line).to_string());
        }
        if trimmed.starts_with("Call log:") {
            call_log = Some(trimmed.to_string());
            in_call_log = true;
            continue;
        }
        if in_call_log
            && call_detail.is_none()
            && !trimmed.is_empty()
            && !trimmed.starts_with("attachment #")
        {
            call_detail = Some(trimmed.to_string());
        }
        if trimmed.starts_with("attachment #") {
            attachment = Some(trimmed.to_string());
            in_call_log = false;
        }
        if attachment_path.is_none()
            && (line.contains("test-results/") || line.contains("trace.zip"))
        {
            attachment_path = Some(trimmed.to_string());
        }
    }

    let result_line = (!result_lines.is_empty()).then(|| result_lines.join("; "));
    let reserved = usize::from(result_line.is_some());
    let available = maximum.saturating_sub(reserved);
    let mut result = Vec::new();

    if result.len() < available {
        result.push(heading);
    }
    if result.len() < available {
        if let Some(line) = error.or(fallback) {
            result.push(line);
        }
    }
    if result.len() < available {
        if let Some(mut line) = call_log {
            if let Some(detail) = call_detail {
                line.push(' ');
                line.push_str(&detail);
            }
            result.push(line);
        }
    }
    if result.len() < available {
        if let Some(mut line) = attachment {
            if let Some(path) = attachment_path {
                line.push(' ');
                line.push_str(&path);
            }
            result.push(line);
        } else if let Some(path) = attachment_path {
            result.push(path);
        }
    }
    if let Some(line) = result_line {
        result.push(line);
    }

    result
}

fn is_failure_heading(line: &str) -> bool {
    let trimmed = line.trim_start();
    let Some((number, rest)) = trimmed.split_once(") ") else {
        return false;
    };
    if number.is_empty() || !number.chars().all(|character| character.is_ascii_digit()) {
        return false;
    }

    has_location(rest, true).is_some() || is_project_heading(rest)
}

fn is_project_heading(rest: &str) -> bool {
    let Some(close_bracket) = rest.find(']') else {
        return false;
    };
    if !rest.starts_with('[') || close_bracket == 1 {
        return false;
    }

    let after_project = rest[close_bracket + 1..].trim_start();
    let Some(after_arrow) = after_project.strip_prefix('›') else {
        return false;
    };
    let Some(location_end) = has_location(after_arrow, false) else {
        return false;
    };

    after_arrow[location_end..].trim_start().starts_with('›')
}

fn has_location(value: &str, require_spec: bool) -> Option<usize> {
    for extension in SCRIPT_EXTENSIONS {
        let marker = if require_spec {
            format!(".spec.{extension}:")
        } else {
            format!(".{extension}:")
        };

        for (index, _) in value.match_indices(&marker) {
            let after_marker = &value[index + marker.len()..];
            let Some((line_number, after_line)) = after_marker.split_once(':') else {
                continue;
            };
            if line_number.is_empty()
                || !line_number
                    .chars()
                    .all(|character| character.is_ascii_digit())
            {
                continue;
            }
            let column_length = after_line
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .count();
            if column_length == 0 {
                continue;
            }

            return Some(
                index + marker.len() + line_number.len() + 1 + column_length,
            );
        }
    }

    None
}

fn is_error_line(line: &str) -> bool {
    if line.starts_with("Error:") || line.starts_with("expect(") {
        return true;
    }

    line.split_once("Error:").is_some_and(|(prefix, _)| {
        !prefix.is_empty()
            && prefix
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
    })
}

fn is_result_summary(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }

    let core = line
        .rfind(" (")
        .filter(|_| line.ends_with(')'))
        .map_or(line, |index| &line[..index]);

    core.split(',').all(|part| {
        let mut words = part.split_whitespace();
        words
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .is_some()
            && matches!(words.next(), Some("passed" | "failed" | "skipped"))
            && words.next().is_none()
    })
}

fn normalize_result_summary(line: &str) -> String {
    line.split(',')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("; ")
}
