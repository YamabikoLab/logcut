# logcut

Reduce AI token usage with concise failure summaries for WordPress development commands.

`logcut` is a Linux command-line tool for AI-assisted WordPress plugin and theme development. It keeps successful command output quiet and extracts the useful parts of failed output, so an AI coding assistant receives a focused result instead of a long stream of routine logs.

It is especially useful with PHPCS, PHPCBF, PHPUnit, PHPStan, `wp-scripts`, Stylelint, Composer, and other commands commonly used in WordPress development. Docker, Git, Jest, Vitest, and the other supported profiles remain available for the surrounding development workflow.

## WordPress development examples

Run the commands you already use through `logcut`:

```bash
logcut --profile=phpcs composer lint:php
logcut --profile=phpcbf composer format:php
logcut --profile=phpunit composer test
logcut --profile=phpstan composer analyse
logcut --profile=stylelint npm run lint:css
logcut npm run build
```

When a command succeeds, `logcut` normally returns only a short result:

```text
Running: composer [1 args]
PASS (2s): composer [1 args]
```

When a command fails, it reports a concise profile-specific summary. By default, it also retains the full failure log after applying best-effort secret masking; use `--no-retain-log` or `LOGCUT_RETAIN_FAILED_LOG=0` to discard the log after summary generation.

## Why use logcut

- Reduce routine command output sent to AI coding assistants.
- Surface the errors and file locations that matter first.
- Use WordPress-focused profiles without changing the underlying commands.
- Preserve the original exit code and choose whether to retain or discard the full failure log.
- Apply best-effort redaction of common secrets to summaries and retained failure logs.

## Behavior

- Prints the command name and argument count before execution.
- Suppresses successful command output and prints a short `PASS` line.
- Preserves a minimal success result for Docker builds and Git transfers.
- Shows a profile-specific failure summary when the command fails.
- Falls back to the tail of the log when no profile-specific summary is available.
- Keeps the full log on failure by default and removes it on success.
- Can discard the full failure log after generating the summary with `--no-retain-log` or `LOGCUT_RETAIN_FAILED_LOG=0`.
- Returns the original command exit code, except for a successful PHPCBF repair reported with exit code `1`.
- Forwards stdin to the child command.
- Forwards `HUP`, `INT`, and `TERM` to the child process group.
- Runs the child command with the caller's original umask.
- Removes terminal escape sequences and unsafe control characters from summaries.
- Redacts common authorization headers, token/password assignments, CI and cloud credential environment variables, and URL credentials from summaries and retained failure logs.
- Creates new log directories with mode `0700`, without changing permissions on existing directories.
- Marks logcut-owned directories with `.logcut-directory` and prunes logs only after validating the directory and marker.
- Rejects existing non-empty directories that cannot be confirmed as logcut-owned.

An existing `LOGCUT_LOG_DIRECTORY` can be initialized only when it is empty, owned by the current user, and already has mode `0700`. Existing non-empty directories require a valid `.logcut-directory` marker owned by the current user with mode `0600`. When secure logging cannot be established, logcut uses its existing direct-execution fallback and does not create, modify, or prune files in that directory.

Secret masking is best effort. It covers the documented common key/value, header, JSON, quoted-value, environment-variable, and URL userinfo forms, but it cannot guarantee detection of unknown formats, arbitrary confidential data, multiline secrets, certificates, or private keys. Avoid printing secrets whenever possible, even when using `logcut`.

`--no-retain-log` and `LOGCUT_RETAIN_FAILED_LOG=0` remove the failure log during normal completion after the summary is generated. They do not guarantee that plaintext is never written to disk or that a log cannot remain after forced termination or an operating-system failure.

## Requirements

- Linux x86_64 or ARM64 with GNU/glibc for the prebuilt binaries
- Linux and Rust 1.70 or later when building from source

## Install from GitHub Release

Download `SHA256SUMS` and the archive matching your system from the `v0.1.16` GitHub Release:

```text
logcut-v0.1.16-x86_64-unknown-linux-gnu.tar.gz
logcut-v0.1.16-aarch64-unknown-linux-gnu.tar.gz
SHA256SUMS
```

Check the machine architecture when needed:

```bash
uname -m
```

Use the `x86_64` archive when the command prints `x86_64`, or the `aarch64` archive when it prints `aarch64` or `arm64`.

Verify the downloaded archive, extract it, and install the binary for the current user. The following example uses the x86_64 archive:

```bash
sha256sum --ignore-missing --check SHA256SUMS
tar -xzf logcut-v0.1.16-x86_64-unknown-linux-gnu.tar.gz
mkdir -p ~/.local/bin
install -m 0755 logcut ~/.local/bin/logcut
```

Make sure `~/.local/bin` is included in `PATH`:

```bash
export PATH="$HOME/.local/bin:$PATH"
command -v logcut
```

Run a harmless command through the installed binary to confirm that it works:

```bash
logcut true
```

Expected output is similar to:

```text
Running: true
PASS (0s): true
```

## Install from Git

When a Rust toolchain is available, install directly from the repository:

```bash
cargo install --git https://github.com/YamabikoLab/logcut.git --tag v0.1.16 --locked
logcut true
```

## Build

```bash
cargo build --release --locked
```

The binary is created at:

```text
target/release/logcut
```

To install the current checkout for the current user:

```bash
cargo install --path . --locked
```

Make sure Cargo's binary directory is included in `PATH`:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

## Usage

```bash
logcut [OPTIONS] <command> [arguments...]
```

Options:

```text
--profile=PROFILE  Select the failure-summary profile (default: auto)
--no-retain-log    Discard the full log after a failure summary
-h, --help         Print help
-V, --version      Print version
```

Examples:

```bash
logcut --version
logcut --help
logcut npm test
logcut --no-retain-log npm test
LOGCUT_RETAIN_FAILED_LOG=0 logcut npm test
logcut --profile=jest npm test
logcut --profile=stylelint npm run lint:css
logcut --profile=phpcs composer lint:php
logcut --profile=webpack npm run build
logcut --profile=playwright npm run test:e2e
logcut docker build -t example/app:dev .
logcut docker compose build web
logcut git push origin main
logcut git pull --ff-only
logcut git fetch --prune
logcut --profile=docker-build sh -c './custom-build-command'
logcut --profile=git-transfer sh -c './custom-git-wrapper'
```

`logcut` options are recognized only before the command. Therefore, `logcut --help` prints logcut's help, while `logcut npm --help` runs `npm --help`.

A successful command normally produces output similar to:

```text
Running: npm [1 args]
PASS (2s): npm [1 args]
```

Docker build and Git transfer commands retain a small amount of useful success information. For example:

```text
Running: git push
PASS (1s): git push
Remote: github.com:YamabikoLab/logcut.git
1111111..2222222  main -> main
```

When a command fails, `logcut` prints a concise summary and, by default, the path to the retained full log. The retained log is rewritten with the same best-effort secret masking used for summaries before its path is reported.

When `--no-retain-log` or `LOGCUT_RETAIN_FAILED_LOG=0` is used, `logcut` still generates the failure summary and preserves the command exit code, then removes the log and prints `Full log discarded.` instead of a path. The CLI option takes precedence over the environment setting.

## Profiles

The default profile is `auto`. Supported profiles are:

| Profile | Description |
| --- | --- |
| `auto` | Detect the profile from the command or command output. |
| `jest` | Summarize Jest test failures. |
| `vitest` | Summarize Vitest failures. |
| `prettier` | Summarize Prettier formatting failures. |
| `eslint` | Summarize ESLint errors. |
| `stylelint` | Summarize Stylelint errors. |
| `typescript` | Summarize TypeScript compiler errors. |
| `phpunit` | Summarize PHPUnit test failures. |
| `phpstan` | Summarize PHPStan analysis errors. |
| `php-lint` | Summarize PHP syntax errors. |
| `phpcs` | Summarize PHP_CodeSniffer violations. |
| `phpcbf` | Summarize PHP Code Beautifier and Fixer results. |
| `contract` | Summarize contract-check failures. |
| `vite` | Summarize Vite build failures. |
| `webpack` | Summarize webpack build failures. |
| `composer` | Summarize Composer failures. |
| `playwright` | Summarize Playwright test failures. |
| `docker-build` | Summarize `docker build` and `docker compose build` results. |
| `git-transfer` | Summarize `git push`, `git pull`, and `git fetch` results. |
| `generic` | Show the tail of the command output. |

Stylelint output for CSS, SCSS, Sass, and Less files is detected automatically, including failures left after `lint-style --fix`. PHPCS, PHPCBF, webpack output from `wp-scripts build`, Docker BuildKit output, and common Git transfer output are also detected automatically. PHPCBF exit code `1` is treated as success only when its result summary reports that errors were fixed and none remain.

For supported Docker and Git commands, `auto` also uses the executable and subcommand. This avoids relying on a generic word such as `build`, `push`, or `fetch` in unrelated output.

The profile can be selected with `--profile=PROFILE` or `LOGCUT_PROFILE`.

### Docker build summaries

The `docker-build` profile removes routine BuildKit progress and keeps the most useful available details, including:

- failed Compose service
- failed build step and Dockerfile location
- `RUN`, `COPY`, and other Dockerfile instructions
- exit code and nearby package/build errors
- successfully built service or image name

It covers `docker build` and `docker compose build`. Commands whose successful output is itself the requested information, such as `docker ps`, `docker images`, `docker logs`, `docker compose ps`, `docker compose logs`, `docker run`, and `docker exec`, remain outside this profile.

### Git transfer summaries

The `git-transfer` profile keeps remotes, branch/ref updates, commit ranges, and results such as new branch, fast-forward, or up to date. Failures are classified when possible, including:

- non-fast-forward rejection
- missing upstream/tracking configuration
- merge conflicts
- authentication, permission, SSH, host-key, repository, branch-policy, hook, network, DNS, and TLS errors

It covers `git push`, `git pull`, and `git fetch`. Information-oriented commands such as `git status`, `git log`, `git diff`, and `git show` remain outside this profile.

Watch, follow, and interactive modes are not targeted by either profile.

## Environment variables

| Variable | Default | Description |
| --- | ---: | --- |
| `LOGCUT_PROFILE` | `auto` | Failure-summary profile. |
| `LOGCUT_SUMMARY_LINES` | `40` | Maximum number of summary lines. |
| `LOGCUT_TAIL_LINES` | `40` | Compatibility fallback used when `LOGCUT_SUMMARY_LINES` is unset. |
| `LOGCUT_MAX_ERRORS` | `20` | Maximum number of errors for profiles that support an error limit. |
| `LOGCUT_LOG_DIRECTORY` | `/tmp/logcut-<uid>` | Dedicated directory used for retained failure logs. Existing non-empty directories require a valid `.logcut-directory` marker. |
| `LOGCUT_LOG_MAX_FILES` | `10` | Maximum number of retained logs. |
| `LOGCUT_LOG_MAX_AGE_DAYS` | `1` | Maximum log age in days. Expired logs are removed on a later `logcut` run. |
| `LOGCUT_RETAIN_FAILED_LOG` | `1` | Set to `0` to discard the full log after a failure summary. |

Invalid positive-integer settings are reported and replaced with their defaults. `LOGCUT_RETAIN_FAILED_LOG` accepts `0` or `1`; invalid values are reported and treated as `1`.

## Development

```bash
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo build --release --locked
```

## Scope

This initial version targets Linux only.

## License

This project is licensed under the GNU General Public License v2.0 or later (`GPL-2.0-or-later`). See [LICENSE](LICENSE).
