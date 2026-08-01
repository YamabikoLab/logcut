pub(crate) fn detect(output: &str) -> bool {
    let mut has_stylesheet = false;
    let mut has_problem = false;

    for line in output.lines() {
        if is_stylesheet_heading(line) {
            has_stylesheet = true;
        } else if is_problem_line(line) {
            has_problem = true;
        }
    }

    has_stylesheet && has_problem
}

pub(crate) fn summarize(output: &str, maximum: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut current_file: Option<&str> = None;
    let mut last_file: Option<&str> = None;
    let mut problem_count = 0usize;

    for line in output.lines() {
        if is_stylesheet_heading(line) {
            current_file = Some(line);
            continue;
        }

        if !is_problem_line(line) {
            continue;
        }

        problem_count += 1;
        if problem_count > maximum {
            continue;
        }

        if current_file != last_file {
            if let Some(file) = current_file {
                result.push(file.to_string());
            }
            last_file = current_file;
        }
        result.push(line.to_string());
    }

    if problem_count > maximum {
        result.push(format!(
            "[{} additional Stylelint problems omitted]",
            problem_count - maximum
        ));
    }

    if let Some(summary) = output.lines().rev().find(|line| is_summary_line(line)) {
        result.push(summary.to_string());
    }

    result
}

fn is_stylesheet_heading(line: &str) -> bool {
    if line.starts_with(char::is_whitespace) {
        return false;
    }

    let trimmed = line.trim();
    [".css", ".scss", ".sass", ".less"]
        .iter()
        .any(|suffix| trimmed.ends_with(suffix))
}

fn is_problem_line(line: &str) -> bool {
    let mut parts = line.split_whitespace();
    let Some(position) = parts.next() else {
        return false;
    };
    let Some(marker) = parts.next() else {
        return false;
    };

    is_position(position) && matches!(marker, "✖" | "×" | "⚠" | "error" | "warning")
}

fn is_position(value: &str) -> bool {
    value.split_once(':').is_some_and(|(line_number, column)| {
        !line_number.is_empty()
            && !column.is_empty()
            && line_number
                .chars()
                .all(|character| character.is_ascii_digit())
            && column.chars().all(|character| character.is_ascii_digit())
    })
}

fn is_summary_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    matches!(trimmed.chars().next(), Some('✖' | '×' | '⚠'))
        && trimmed.contains("problem")
        && (trimmed.contains("error") || trimmed.contains("warning"))
}
