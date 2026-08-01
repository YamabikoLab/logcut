use crate::summary::Profile;

pub(crate) fn successful_nonzero_exit(profile: Profile, status: i32, output: &str) -> bool {
    crate::summary::successful_nonzero_exit(profile, status, output)
        && summary_has_no_remaining_errors(output)
}

fn summary_has_no_remaining_errors(output: &str) -> bool {
    let mut in_summary = false;
    let mut found_file = false;

    for line in output.lines() {
        if line.contains("PHPCBF RESULT SUMMARY") {
            in_summary = true;
            continue;
        }
        if !in_summary {
            continue;
        }
        if line.contains("A TOTAL OF ") {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('-') || trimmed.starts_with("FILE") {
            continue;
        }

        let mut columns = trimmed.split_whitespace().rev();
        let remaining = columns.next().and_then(|value| value.parse::<usize>().ok());
        let fixed = columns.next().and_then(|value| value.parse::<usize>().ok());
        if let (Some(remaining), Some(_)) = (remaining, fixed) {
            found_file = true;
            if remaining != 0 {
                return false;
            }
        }
    }

    found_file
}
