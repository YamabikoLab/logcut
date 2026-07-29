# logcut

`logcut` is a Linux command-line tool that runs another command quietly and prints only a short result.

It is a Rust port of the existing Bash `quiet-run` command used in `yamabiko-flow-blocks`. The first implementation intentionally follows the existing behavior instead of adding new features.

## Behavior

- Prints the command name and argument count before execution.
- Suppresses successful command output and prints a short `PASS` line.
- Shows a profile-specific failure summary when the command fails.
- Falls back to the tail of the log when no profile-specific summary is available.
- Keeps the full log on failure and removes it on success.
- Returns the original command exit code.
- Forwards stdin to the child command.
- Forwards `HUP`, `INT`, and `TERM` to the child process group.
- Removes ANSI escape sequences, OSC hyperlinks, and carriage returns from summaries.
- Restricts the log directory to mode `0700` and prunes old logs.

## Requirements

- Linux
- Rust 1.70 or later when building from source

## Build

```bash
cargo build --release
```

The binary is created at:

```text
target/release/logcut
```

To install it for the current user:

```bash
cargo install --path .
```

Make sure Cargo's binary directory is included in `PATH`:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

## Usage

```bash
logcut [--profile=PROFILE] <command> [arguments...]
```

Examples:

```bash
logcut npm test
logcut --profile=typescript npm run typecheck
logcut --profile=playwright npm run test:e2e
```

A successful command produces output similar to:

```text
Running: npm [1 args]
PASS (2s): npm [1 args]
```

When a command fails, `logcut` prints a concise summary and the path to the retained full log.

## Profiles

The default profile is `auto`. Supported profiles are:

- `auto`
- `vitest`
- `prettier`
- `eslint`
- `typescript`
- `phpunit`
- `phpstan`
- `php-lint`
- `contract`
- `vite`
- `composer`
- `playwright`
- `generic`

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
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## Scope

This initial version targets Linux only and does not include features that were not present in the original `quiet-run` implementation.
