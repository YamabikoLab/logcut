use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const REDACTED: &[u8] = b" [REDACTED]";
const REDACTED_MARKER: &[u8] = b"[REDACTED]";
const SENSITIVE_COMPONENTS: [&[u8]; 9] = [
    b"TOKEN",
    b"SECRET",
    b"PASSWORD",
    b"PASS",
    b"API_KEY",
    b"ACCESS_KEY",
    b"PRIVATE_KEY",
    b"CREDENTIALS",
    b"AUTH",
];
const FIXED_SENSITIVE_KEYS: [&[u8]; 4] = [
    b"AZURE_DEVOPS_EXT_PAT",
    b"AZURE_STORAGE_KEY",
    b"GOOGLE_CLOUD_KEYFILE_JSON",
    b"GCLOUD_SERVICE_KEY",
];

pub(super) fn redact_log_file(path: &Path) -> io::Result<()> {
    crate::logging::redact_log_file(path)?;

    let (temporary_path, temporary_file) = create_temporary_file(path)?;
    let result = redact_file_to(path, &temporary_path, temporary_file);
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn create_temporary_file(path: &Path) -> io::Result<(PathBuf, File)> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("command.log");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    for attempt in 0..1000u32 {
        let temporary_path = directory.join(format!(
            ".{name}.expanded-redaction.{}.{}.{}.tmp",
            std::process::id(),
            nanos,
            attempt
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary_path)
        {
            Ok(file) => return Ok((temporary_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a temporary expanded-redaction file",
    ))
}

fn redact_file_to(path: &Path, temporary_path: &Path, temporary_file: File) -> io::Result<()> {
    let source = File::open(path)?;
    let length = source.metadata()?.len();
    let mut reader = BufReader::new(source.take(length));
    let mut writer = BufWriter::new(temporary_file);
    let mut line = Vec::new();

    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        write_redacted_line(&line, &mut writer)?;
    }

    writer.flush()?;
    let file = writer.into_inner().map_err(|error| error.into_error())?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary_path, path)
}

fn write_redacted_line<W: Write>(line: &[u8], writer: &mut W) -> io::Result<()> {
    let redacted = redact_line(line);
    writer.write_all(redacted.as_deref().unwrap_or(line))
}

fn redact_line(line: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(line.len());
    let mut copied_until = 0usize;
    let mut scan = 0usize;
    let mut changed = false;

    while scan < line.len() {
        if !matches!(line[scan], b':' | b'=') {
            scan += 1;
            continue;
        }

        let separator = scan;
        let Some((key_start, key_end)) = key_bounds(line, separator) else {
            scan += 1;
            continue;
        };
        if !is_sensitive_key(&line[key_start..key_end]) {
            scan += 1;
            continue;
        }

        let value_start = separator + 1;
        let mut content_start = value_start;
        while content_start < line.len() && line[content_start].is_ascii_whitespace() {
            content_start += 1;
        }
        if content_start >= line.len() {
            break;
        }
        let allow_structural_boundary =
            line[separator] == b':' && is_quoted_key(line, key_start, separator);
        if let Some(marker_end) =
            existing_redaction_end(line, content_start, allow_structural_boundary)
        {
            scan = marker_end;
            continue;
        }

        let value_end = value_end(line, content_start, allow_structural_boundary);
        output.extend_from_slice(&line[copied_until..value_start]);
        output.extend_from_slice(REDACTED);
        copied_until = value_end;
        scan = value_end.max(value_start + 1);
        changed = true;
    }

    if !changed {
        return None;
    }

    output.extend_from_slice(&line[copied_until..]);
    Some(output)
}

fn key_bounds(line: &[u8], separator: usize) -> Option<(usize, usize)> {
    let mut key_end = separator;
    while key_end > 0 && line[key_end - 1].is_ascii_whitespace() {
        key_end -= 1;
    }
    if key_end > 0 && matches!(line[key_end - 1], b'\'' | b'"') {
        key_end -= 1;
    }

    let mut key_start = key_end;
    while key_start > 0
        && (line[key_start - 1].is_ascii_alphanumeric()
            || matches!(line[key_start - 1], b'_' | b'-'))
    {
        key_start -= 1;
    }

    (key_start < key_end).then_some((key_start, key_end))
}

fn is_quoted_key(line: &[u8], key_start: usize, separator: usize) -> bool {
    let mut key_end = separator;
    while key_end > key_start && line[key_end - 1].is_ascii_whitespace() {
        key_end -= 1;
    }
    if key_end == 0 {
        return false;
    }

    let quote = line[key_end - 1];
    matches!(quote, b'\'' | b'"') && key_start > 0 && line[key_start - 1] == quote
}

fn existing_redaction_end(
    line: &[u8],
    content_start: usize,
    allow_structural_boundary: bool,
) -> Option<usize> {
    if !line[content_start..].starts_with(REDACTED_MARKER) {
        return None;
    }

    let marker_end = content_start + REDACTED_MARKER.len();
    let line_end = line[content_start..]
        .iter()
        .position(|byte| matches!(byte, b'\r' | b'\n'))
        .map_or(line.len(), |offset| content_start + offset);

    if marker_end == line_end
        || (allow_structural_boundary && is_structural_value_boundary(line[marker_end]))
    {
        return Some(marker_end);
    }
    if !line[marker_end].is_ascii_whitespace() {
        return None;
    }

    let mut boundary = marker_end;
    while boundary < line_end && line[boundary].is_ascii_whitespace() {
        boundary += 1;
    }

    if boundary == line_end
        || (allow_structural_boundary && is_structural_value_boundary(line[boundary]))
        || starts_sensitive_assignment(line, marker_end, line_end)
    {
        Some(marker_end)
    } else {
        None
    }
}

fn is_structural_value_boundary(byte: u8) -> bool {
    matches!(byte, b',' | b';' | b'}' | b']' | b')' | b'\'' | b'"')
}

fn is_quoted_value_boundary(line: &[u8], boundary: usize, line_end: usize) -> bool {
    match line[boundary] {
        b',' => starts_quoted_mapping_key(line, boundary + 1, line_end),
        b'}' | b']' => line[boundary..line_end]
            .iter()
            .all(|byte| byte.is_ascii_whitespace() || matches!(byte, b'}' | b']')),
        _ => false,
    }
}

fn starts_quoted_mapping_key(line: &[u8], mut index: usize, line_end: usize) -> bool {
    while index < line_end && line[index].is_ascii_whitespace() {
        index += 1;
    }

    let Some(quote @ (b'\'' | b'"')) = line.get(index).copied() else {
        return false;
    };
    index += 1;

    let mut escaped = false;
    while index < line_end {
        let byte = line[index];
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == quote {
            index += 1;
            while index < line_end && line[index].is_ascii_whitespace() {
                index += 1;
            }
            return index < line_end && line[index] == b':';
        }
        index += 1;
    }

    false
}

fn value_end(line: &[u8], content_start: usize, allow_structural_boundary: bool) -> usize {
    let line_end = line[content_start..]
        .iter()
        .position(|byte| matches!(byte, b'\r' | b'\n'))
        .map_or(line.len(), |offset| content_start + offset);

    if allow_structural_boundary {
        if let Some(quote @ (b'\'' | b'"')) = line.get(content_start).copied() {
            let mut index = content_start + 1;
            let mut escaped = false;
            while index < line_end {
                let byte = line[index];
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == quote {
                    let mut boundary = index + 1;
                    while boundary < line_end && line[boundary].is_ascii_whitespace() {
                        boundary += 1;
                    }
                    if boundary == line_end || is_quoted_value_boundary(line, boundary, line_end) {
                        return index + 1;
                    }
                    return line_end;
                }
                index += 1;
            }
            return line_end;
        }
    }

    let mut index = content_start;
    while index < line_end {
        if line[index].is_ascii_whitespace() {
            if starts_sensitive_assignment(line, index, line_end) {
                return index;
            }
            while index < line_end && line[index].is_ascii_whitespace() {
                index += 1;
            }
        } else {
            index += 1;
        }
    }
    line_end
}

fn starts_sensitive_assignment(line: &[u8], mut index: usize, line_end: usize) -> bool {
    while index < line_end && line[index].is_ascii_whitespace() {
        index += 1;
    }

    let key_start = index;
    while index < line_end
        && (line[index].is_ascii_alphanumeric() || matches!(line[index], b'_' | b'-'))
    {
        index += 1;
    }
    if key_start == index || !is_sensitive_key(&line[key_start..index]) {
        return false;
    }

    while index < line_end && line[index].is_ascii_whitespace() {
        index += 1;
    }
    index < line_end && matches!(line[index], b':' | b'=')
}

fn is_sensitive_key(key: &[u8]) -> bool {
    FIXED_SENSITIVE_KEYS
        .iter()
        .any(|candidate| key.eq_ignore_ascii_case(candidate))
        || SENSITIVE_COMPONENTS.iter().any(|component| {
            key.windows(component.len())
                .any(|window| sensitive_component_matches(window, component))
        })
}

fn sensitive_component_matches(candidate: &[u8], component: &[u8]) -> bool {
    candidate
        .iter()
        .zip(component.iter())
        .all(|(&candidate, &component)| {
            let candidate = if candidate == b'-' {
                b'_'
            } else {
                candidate.to_ascii_uppercase()
            };
            let component = if component == b'-' {
                b'_'
            } else {
                component.to_ascii_uppercase()
            };
            candidate == component
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CI_AND_CLOUD_KEYS: [&str; 45] = [
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
        "CI_JOB_TOKEN",
        "CI_DEPLOY_PASSWORD",
        "CI_REGISTRY_PASSWORD",
        "CIRCLE_OIDC_TOKEN",
        "CIRCLE_OIDC_TOKEN_V2",
        "CIRCLE_TOKEN",
        "BITBUCKET_STEP_OIDC_TOKEN",
        "BITBUCKET_PACKAGES_TOKEN",
        "SYSTEM_ACCESSTOKEN",
        "AZURE_DEVOPS_EXT_PAT",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_SESSION_TOKEN",
        "AWS_SECURITY_TOKEN",
        "AWS_CONTAINER_AUTHORIZATION_TOKEN",
        "AWS_ACCESS_KEY_ID",
        "AZURE_CLIENT_SECRET",
        "AZURE_CLIENT_CERTIFICATE_PASSWORD",
        "ARM_CLIENT_SECRET",
        "ARM_CLIENT_CERTIFICATE_PASSWORD",
        "AZURE_STORAGE_KEY",
        "CLOUDSDK_AUTH_ACCESS_TOKEN",
        "GOOGLE_CREDENTIALS",
        "GOOGLE_CLOUD_KEYFILE_JSON",
        "GCLOUD_SERVICE_KEY",
        "CLOUDFLARE_API_TOKEN",
        "CLOUDFLARE_API_KEY",
        "CF_API_TOKEN",
        "CF_API_KEY",
        "DIGITALOCEAN_ACCESS_TOKEN",
        "DIGITALOCEAN_TOKEN",
        "DO_API_TOKEN",
        "DO_API_KEY",
        "DO_OAUTH_TOKEN",
        "NETLIFY_AUTH_TOKEN",
        "VERCEL_TOKEN",
        "HEROKU_API_KEY",
        "FLY_API_TOKEN",
        "NPM_TOKEN",
        "NODE_AUTH_TOKEN",
        "YARN_NPM_AUTH_TOKEN",
        "DOCKERHUB_TOKEN",
        "DOCKER_PASSWORD",
    ];

    #[test]
    fn redacts_major_ci_and_cloud_environment_variables() {
        for key in CI_AND_CLOUD_KEYS {
            let line = format!("{key}=secret-value\n");
            let redacted = redact_line(line.as_bytes()).unwrap();
            assert!(!redacted.windows(12).any(|value| value == b"secret-value"));
            assert!(redacted
                .windows(b"[REDACTED]".len())
                .any(|value| value == b"[REDACTED]"));
        }
    }

    #[test]
    fn redacts_registry_and_package_environment_variables() {
        for key in [
            "DOCKER_AUTH_CONFIG",
            "TWINE_PASSWORD",
            "PYPI_API_TOKEN",
            "CARGO_REGISTRY_TOKEN",
            "CARGO_REGISTRIES_PRIVATE_TOKEN",
            "GEM_HOST_API_KEY",
            "RUBYGEMS_API_KEY",
            "COMPOSER_AUTH",
        ] {
            let line = format!("{key}=secret-value\n");
            let redacted = redact_line(line.as_bytes()).unwrap();
            assert!(!redacted.windows(12).any(|value| value == b"secret-value"));
        }
    }

    #[test]
    fn redacts_generic_patterns_case_insensitively() {
        let line = b"my_api_token=one DATABASE_PASSWORD=two service_client_secret=three DEPLOY_PRIVATE_KEY=four REGISTRY_AUTH=five\n";
        let redacted = redact_line(line).unwrap();

        for secret in [b"one".as_slice(), b"two", b"three", b"four", b"five"] {
            assert!(!redacted.windows(secret.len()).any(|value| value == secret));
        }
        assert_eq!(
            redacted
                .windows(b"[REDACTED]".len())
                .filter(|value| *value == b"[REDACTED]")
                .count(),
            5
        );
    }

    #[test]
    fn redacts_unquoted_values_through_line_end() {
        let spaced = redact_line(b"MY_SECRET=correct horse battery staple\n").unwrap();
        assert_eq!(spaced, b"MY_SECRET= [REDACTED]\n".to_vec());

        let json = redact_line(
            b"GOOGLE_CLOUD_KEYFILE_JSON={\"first\":\"safe\",\"credential\":\"leaky secret\"}\n",
        )
        .unwrap();
        assert_eq!(json, b"GOOGLE_CLOUD_KEYFILE_JSON= [REDACTED]\n".to_vec());
    }

    #[test]
    fn redacts_quoted_values_through_assignment_boundary() {
        let concatenated = redact_line(b"OPENAI_API_KEY=\"sk-prefix\"secret-suffix\n").unwrap();
        assert_eq!(concatenated, b"OPENAI_API_KEY= [REDACTED]\n".to_vec());

        let quoted = redact_line(b"MY_SECRET=\"secret-value\"\n").unwrap();
        assert_eq!(quoted, b"MY_SECRET= [REDACTED]\n".to_vec());

        let multiple =
            redact_line(b"OPENAI_API_KEY=\"first-secret\" OTHER_TOKEN='second-secret'\n").unwrap();
        assert_eq!(
            multiple,
            b"OPENAI_API_KEY= [REDACTED] OTHER_TOKEN= [REDACTED]\n".to_vec()
        );

        let json = redact_line(b"{\"password\":\"one\",\"api-key\":\"two\"}\n").unwrap();
        assert_eq!(
            json,
            b"{\"password\": [REDACTED],\"api-key\": [REDACTED]}\n".to_vec()
        );
    }

    #[test]
    fn redacts_suffix_after_quoted_colon_value() {
        let concatenated =
            redact_line(b"{\"OPENAI_API_KEY\":\"sk-prefix\"secret-suffix}\n").unwrap();
        assert_eq!(concatenated, b"{\"OPENAI_API_KEY\": [REDACTED]\n".to_vec());

        let delimited =
            redact_line(b"{\"OPENAI_API_KEY\":\"sk-prefix\",delimiter-suffix}\n").unwrap();
        assert_eq!(delimited, b"{\"OPENAI_API_KEY\": [REDACTED]\n".to_vec());

        let json = redact_line(
            b"{\"OPENAI_API_KEY\":\"first-secret\",\"OTHER_TOKEN\":\"second-secret\"}\n",
        )
        .unwrap();
        assert_eq!(
            json,
            b"{\"OPENAI_API_KEY\": [REDACTED],\"OTHER_TOKEN\": [REDACTED]}\n".to_vec()
        );
    }

    #[test]
    fn redacts_values_with_redacted_prefix_and_trailing_secret() {
        let spaced = redact_line(b"MY_SECRET=[REDACTED] actual-secret\n").unwrap();
        assert_eq!(spaced, b"MY_SECRET= [REDACTED]\n".to_vec());

        let delimited = redact_line(b"MY_SECRET=[REDACTED],actual-secret\n").unwrap();
        assert_eq!(delimited, b"MY_SECRET= [REDACTED]\n".to_vec());
    }

    #[test]
    fn preserves_following_sensitive_assignment_boundaries() {
        let redacted = redact_line(b"MY_SECRET=correct horse OTHER_TOKEN=second value\n").unwrap();
        assert_eq!(
            redacted,
            b"MY_SECRET= [REDACTED] OTHER_TOKEN= [REDACTED]\n".to_vec()
        );
    }

    #[test]
    fn preserves_existing_redactions() {
        let line = b"access_token= [REDACTED]\n{\"password\": [REDACTED],\"api-key\": [REDACTED]}\nAuthorization: [REDACTED]\n";
        assert!(redact_line(line).is_none());
    }

    #[test]
    fn does_not_apply_path_or_url_exclusions() {
        for key in [
            "ACTIONS_ID_TOKEN_REQUEST_URL",
            "AWS_WEB_IDENTITY_TOKEN_FILE",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "CLOUDSDK_AUTH_ACCESS_TOKEN_FILE",
            "ARM_CLIENT_SECRET_FILE_PATH",
        ] {
            let line = format!("{key}=/path/to/value\n");
            let redacted = redact_line(line.as_bytes()).unwrap();
            assert!(!redacted
                .windows(b"/path/to/value".len())
                .any(|value| value == b"/path/to/value"));
        }
    }

    #[test]
    fn preserves_unmatched_invalid_utf8() {
        let line = b"NORMAL_OUTPUT=ok\xff\n";
        assert!(redact_line(line).is_none());
    }
}
