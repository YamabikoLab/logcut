use crate::summary::Profile;

pub(crate) fn successful_nonzero_exit(profile: Profile, status: i32, output: &str) -> bool {
    profile == Profile::Phpcbf
        && status == 1
        && output.contains("PHPCBF RESULT SUMMARY")
        && output.contains("A TOTAL OF ")
        && output.contains("ERRORS WERE FIXED")
        && !output.contains("FAILED TO FIX")
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

#[cfg(test)]
mod tests {
    use super::*;

    const SUCCESSFUL_SUMMARY: &str = "PHPCBF RESULT SUMMARY\nFILE FIXED REMAINING\nexample.php 2 0\nA TOTAL OF 2 ERRORS WERE FIXED";

    #[test]
    fn only_exit_code_one_can_be_converted_to_success() {
        assert!(successful_nonzero_exit(
            Profile::Phpcbf,
            1,
            SUCCESSFUL_SUMMARY
        ));
        assert!(!successful_nonzero_exit(
            Profile::Phpcbf,
            70,
            SUCCESSFUL_SUMMARY
        ));
    }
}
