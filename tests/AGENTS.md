# Integration Test Guidance

These instructions apply to tests under `tests/`.
Follow the repository-level guidance as well.

## Test Shape

- Test the CLI as an external process, as a user runs it.
- Keep each test focused on one behavior or one closely related behavior group.
- Use small fixtures that resemble real command output.
- Do not depend on network access or external services.
- Avoid timing-sensitive assertions and unnecessary sleeps.

## Isolation

- Give each test a unique temporary directory.
- Do not rely on shared mutable state or test execution order.
- Keep test-created logs and files inside the test directory.
- Clean up when practical, while preserving useful failure evidence when needed.
- Do not use real credentials, tokens, or private data in fixtures.

## Assertions

- Check exit codes when they are part of the contract.
- Check stdout and stderr separately when their distinction matters.
- Verify log retention, deletion, and permissions when changing logging behavior.
- Keep regression coverage that command arguments and secret-like values are not printed.
- Test signal and child-process behavior without leaving processes behind.
- When changing a profile, cover its key marker and bounded summary output.

## Rust Test Code

- `unwrap()` and `expect()` are acceptable for test setup and assertions.
- Prefer clear setup helpers over elaborate test frameworks.
- Include enough assertion context to identify the failing profile or scenario.
- Avoid duplicating production parsing logic inside tests.

Run the full locked test suite before completing changes:

```bash
cargo test --locked
```
