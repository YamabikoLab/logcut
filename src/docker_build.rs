const INSTRUCTIONS: [&str; 11] = [
    "ADD",
    "ARG",
    "COPY",
    "ENTRYPOINT",
    "ENV",
    "EXPOSE",
    "FROM",
    "HEALTHCHECK",
    "RUN",
    "USER",
    "WORKDIR",
];

pub(crate) fn summarize(output: &str, maximum: usize, max_errors: usize) -> Vec<String> {
    let lines: Vec<&str> = output.lines().collect();
    let errors: Vec<String> = lines
        .iter()
        .filter(|line| is_relevant_error(line))
        .map(|line| line.trim().to_string())
        .collect();
    let mut result = Vec::new();

    if let Some(primary) = errors.last() {
        push_unique(&mut result, primary.clone(), maximum);
    }

    if let Some(service) = lines.iter().rev().find_map(|line| extract_service(line)) {
        push_unique(&mut result, format!("Service: {service}"), maximum);
    }

    if let Some(step) = lines.iter().rev().find_map(|line| extract_step(line)) {
        push_unique(&mut result, format!("Step: {step}"), maximum);
    }

    if let Some(location) = lines
        .iter()
        .rev()
        .find_map(|line| dockerfile_location(line))
    {
        push_unique(&mut result, location, maximum);
    }

    if let Some(instruction) = lines
        .iter()
        .rev()
        .find_map(|line| extract_instruction(line))
    {
        push_unique(&mut result, format!("Instruction: {instruction}"), maximum);
    }

    let error_limit = max_errors.min(maximum.saturating_sub(result.len()));
    let start = errors.len().saturating_sub(error_limit);
    for line in &errors[start..] {
        push_unique(&mut result, line.clone(), maximum);
    }

    result
}

pub(crate) fn summarize_success(output: &str, maximum: usize) -> Vec<String> {
    let mut result = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Successfully built ") || trimmed.starts_with("Successfully tagged ")
        {
            push_unique(&mut result, trimmed.to_string(), maximum);
            continue;
        }

        if let Some(image) = trimmed
            .find("naming to ")
            .map(|index| trimmed[index + "naming to ".len()..].trim_end_matches(" done"))
        {
            if !image.is_empty() {
                push_unique(&mut result, format!("Image: {image}"), maximum);
            }
            continue;
        }

        if let Some(service) = compose_built_service(trimmed) {
            push_unique(&mut result, format!("Service built: {service}"), maximum);
        }
    }

    if result.is_empty() && maximum > 0 {
        result.push("Docker build completed successfully.".to_string());
    }

    result
}

fn extract_service(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    if let Some(index) = lower.find("service \"") {
        let rest = &line[index + "service \"".len()..];
        if let Some(end) = rest.find('"') {
            return Some(rest[..end].to_string());
        }
    }

    let step = extract_step(line)?;
    let open = step.find('[')?;
    let close = step[open + 1..].find(']')? + open + 1;
    let first = step[open + 1..close].split_whitespace().next()?;
    if first.chars().all(|character| character.is_ascii_digit())
        || matches!(
            first,
            "internal" | "auth" | "context" | "exporting" | "base" | "stage-0"
        )
    {
        None
    } else {
        Some(first.to_string())
    }
}

fn extract_step(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if !is_buildkit_step(trimmed) {
        return None;
    }

    let start = trimmed.find('[').unwrap_or(0);
    let step = trimmed[start..]
        .trim_end_matches(" CACHED")
        .trim_end_matches(" DONE")
        .trim();
    if step.is_empty() {
        None
    } else {
        Some(step.to_string())
    }
}

fn is_buildkit_step(line: &str) -> bool {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix('#') {
        return rest
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .count()
            > 0
            && trimmed.contains('[')
            && trimmed.contains(']');
    }

    trimmed.starts_with('>') && trimmed.contains('[') && trimmed.contains(']')
}

fn dockerfile_location(line: &str) -> Option<String> {
    let trimmed = line.trim();
    for name in ["Dockerfile:", "Containerfile:"] {
        if let Some(index) = trimmed.find(name) {
            let location = trimmed[index + name.len()..].trim();
            if !location.is_empty()
                && location
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_digit())
            {
                return Some(format!("{name}{location}"));
            }
        }
    }
    None
}

fn extract_instruction(line: &str) -> Option<String> {
    let trimmed = line.trim();
    for instruction in INSTRUCTIONS {
        let marker = format!("{instruction} ");
        if trimmed.starts_with(&marker) {
            return Some(trimmed.to_string());
        }
        if let Some(index) = trimmed.find(&format!("] {marker}")) {
            return Some(trimmed[index + 2..].to_string());
        }
    }
    None
}

fn is_relevant_error(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || is_progress_noise(trimmed) {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    lower.contains("error:")
        || lower.starts_with("error ")
        || lower.starts_with("fatal:")
        || lower.starts_with("e: ")
        || lower.contains("npm err!")
        || lower.contains("npm error")
        || lower.contains("failed to solve")
        || lower.contains("failed to compute cache key")
        || lower.contains("failed to read dockerfile")
        || lower.contains("failed to build")
        || lower.contains("did not complete successfully")
        || lower.contains("executor failed running")
        || lower.contains("exit code")
        || lower.contains("no such file")
        || lower.contains("not found")
        || lower.contains("permission denied")
        || lower.contains("unable to locate package")
}

fn is_progress_noise(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    is_buildkit_step(line)
        && (lower.ends_with(" done")
            || lower.ends_with(" cached")
            || lower.contains("transferring context:")
            || lower.contains("transferring dockerfile:"))
}

fn compose_built_service(line: &str) -> Option<&str> {
    if line.starts_with('#') || !line.ends_with(" Built") {
        return None;
    }
    let service = line[..line.len() - " Built".len()].trim();
    (!service.is_empty()).then_some(service)
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
    fn summarizes_failed_run_step() {
        let output = "#8 [web 4/5] RUN npm ci\n#8 1.2 npm ERR! dependency failed\nDockerfile:17\nERROR: failed to solve: process exited with exit code: 1\n";
        let summary = summarize(output, 20, 10);
        assert!(summary.iter().any(|line| line == "Service: web"));
        assert!(summary.iter().any(|line| line.contains("RUN npm ci")));
        assert!(summary.iter().any(|line| line.contains("Dockerfile:17")));
        assert!(summary.iter().any(|line| line.contains("exit code: 1")));
    }
}
