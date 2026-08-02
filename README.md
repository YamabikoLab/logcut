# logcut

`logcut` is a Linux command-line tool that keeps command output quiet and shows only a concise result.  
When a command fails, it extracts the important parts of the output and presents an easy-to-read failure summary.

## Behavior

- Prints the command name and argument count before execution.
- Suppresses successful command output and prints a short `PASS` line.
- Shows a profile-specific failure summary when the command fails.
- Falls back to the tail of the log when no profile-specific summary is available.
- Keeps the full log on failure and removes it on success.
- Returns the original command exit code, except for a successful PHPCBF repair reported with exit code `1`.
- Forwards stdin to the child command.
- Forwards `HUP`, `INT`, and `TERM` to the child process group.
- Runs the child command with the caller's original umask.
- Removes terminal escape sequences and unsafe control characters from summaries.
- Restricts the log directory to mode `0700` and prunes old logs.

## Requirements

- Linux x86_64 with GNU/glibc for the prebuilt binary
- Linux and Rust 1.70 or later when building from source

## Install from GitHub Release

Download both of these files from the `v0.1.9` GitHub Release:

```text
logcut-v0.1.9-x86_64-unknown-linux-gnu.tar.gz
SHA256SUMS
```

Verify the archive, extract it, and install the binary for the current user:

```bash
sha256sum --check SHA256SUMS
tar -xzf logcut-v0.1.9-x86_64-unknown-linux-gnu.tar.gz
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
cargo install --git https://github.com/YamabikoLab/logcut.git --tag v0.1.9 --locked
logcut true
```

Because this repository is private, Git authentication must already be configured for the environment running `cargo install`.

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
```

`logcut` options are recognized only before the command. Therefore, `logcut --help` prints logcut's help, while `logcut npm --help` runs `npm --help`.

A successful command produces output similar to:

```text
Running: npm [1 args]
PASS (2s): npm [1 args]
```

When a command fails, `logcut` prints a concise summary and the path to the retained full log.

## Profiles

The default profile is `auto`. Supported profiles are:

| Profile | Description |
| --- | --- |
| `auto` | Detect the profile from command output. |
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
| `generic` | Show the tail of the command output. |

Stylelint output for CSS, SCSS, Sass, and Less files is detected automatically, including failures left after `lint-style --fix`. PHPCS, PHPCBF, and webpack output from `wp-scripts build` are also detected automatically. PHPCBF exit code `1` is treated as success only when its result summary reports that errors were fixed and none remain.

The profile can be selected with `--profile=PROFILE` or `LOGCUT_PROFILE`.

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
