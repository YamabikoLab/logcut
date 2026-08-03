# logcut

`logcut` is a Linux command-line tool that keeps command output quiet and shows only a concise result.  
When a command fails, it extracts the important parts of the output and presents an easy-to-read failure summary.

## Behavior

- Prints the command name and argument count before execution.
- Suppresses successful command output and prints a short `PASS` line.
- Preserves a minimal success result for Docker builds and Git transfers.
- Shows a profile-specific failure summary when the command fails.
- Falls back to the tail of the log when no profile-specific summary is available.
- Keeps the full log on failure and removes it on success.
- Returns the original command exit code, except for a successful PHPCBF repair reported with exit code `1`.
- Forwards stdin to the child command.
- Forwards `HUP`, `INT`, and `TERM` to the child process group.
- Runs the child command with the caller's original umask.
- Removes terminal escape sequences and unsafe control characters from summaries.
- Redacts common authorization headers, token/password assignments, and URL credentials from summaries and retained failure logs.
- Restricts the log directory to mode `0700` and prunes old logs.

Secret masking is best effort. It covers the documented common key/value, header, JSON, quoted-value, and URL userinfo forms, but it cannot guarantee detection of unknown formats, arbitrary confidential data, multiline secrets, certificates, or private keys. Avoid printing secrets whenever possible, even when using `logcut`.

## Requirements

- Linux x86_64 or ARM64 with GNU/glibc for the prebuilt binaries
- Linux and Rust 1.70 or later when building from source

## Install from GitHub Release

Download `SHA256SUMS` and the archive matching your system from the `v0.1.12` GitHub Release:

```text
logcut-v0.1.12-x86_64-unknown-linux-gnu.tar.gz
logcut-v0.1.12-aarch64-unknown-linux-gnu.tar.gz
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
tar -xzf logcut-v0.1.12-x86_64-unknown-linux-gnu.tar.gz
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
cargo install --git https://github.com/YamabikoLab/logcut.git --tag v0.1.12 --locked
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
-h, --help         Print help
-V, --version      Print version
```

Examples:

```bash
logcut --version
logcut --help
logcut npm test
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

When a command fails, `logcut` prints a concise summary and the path to the retained full log. The retained log is rewritten with the same best-effort secret masking used for summaries before its path is reported.

## Profiles

The default profile is `auto`. Supported profiles are:

| Profile | Description |
| --- | --- |
| `auto` | Detect the profile from the command or command output. |
| `jest` | Summarize Jest test failures. |
| `vitest` | Summarize Vitest test failures. |
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
| `LOGCUT_LOG_DIRECTORY` | `/tmp/logcut-<uid>` | Directory used for retained failure logs. |
| `LOGCUT_LOG_MAX_FILES` | `10` | Maximum number of retained logs. |
| `LOGCUT_LOG_MAX_AGE_DAYS` | `7` | Maximum log age in days. |

Invalid positive-integer settings are reported and replaced with their defaults.

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
