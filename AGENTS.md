# Repository Guidance

These instructions apply to the entire repository.
More specific `AGENTS.md` files add only directory-specific guidance.

## Scope

- Keep `logcut` a small command-line tool.
- Preserve existing CLI behavior unless the task explicitly changes it.
- Treat the README and implemented behavior as the current source of truth.
- Do not add speculative features, frameworks, or abstractions.
- Keep changes focused and easy to review.

## Toolchain

- Maintain compatibility with Rust 1.70 or later.
- Use Rust 2021 Edition.
- Prefer the standard library over new dependencies.
- Add a dependency only when it clearly reduces risk or complexity.
- Keep `Cargo.lock` committed and use locked builds.

## Repository Files

- Follow existing style in Rust, TOML, YAML, Shell, and Markdown files.
- Keep Cargo and workflow configuration direct and explicit.
- In workflow Shell, fail on errors and avoid masking command failures.
- Do not print secrets, tokens, credentials, or sensitive environment values.
- Do not commit generated artifacts, retained logs, or local credentials.
- Update documentation only when user-facing behavior or commands change.

## Change Discipline

- Avoid unrelated cleanup while implementing a focused task.
- Do not rename public commands, profiles, or environment variables casually.
- Preserve supported exit codes and documented defaults.
- Prefer a small direct implementation over a reusable system without a current need.
- Add or update tests when observable behavior changes.

## Validation

Run the applicable checks before completing a change:

```bash
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo build --release --locked
```

Report any check that could not be run and the reason.
