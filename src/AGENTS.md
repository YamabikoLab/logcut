# Rust Source Guidance

These instructions apply to production Rust code under `src/`.
Follow the repository-level guidance as well.

## Design

- Use only features available in Rust 1.70.
- Prefer small functions and modules with clear responsibilities.
- Avoid unnecessary traits, generics, indirection, and module layers.
- Keep operating-system behavior explicit rather than hiding it behind broad abstractions.
- Preserve the current Linux and Unix process model unless the task changes it.

## Errors and Data

- Return errors for expected failures instead of using `panic!` or `unwrap()`.
- Keep command arguments and paths as `OsString` or `OsStr` where possible.
- Do not assume that operating-system strings are valid UTF-8.
- Use lossy conversion only for controlled display output.
- Preserve documented defaults and exit-code behavior.

## Unsafe and Process Handling

- Keep `unsafe` blocks and FFI declarations as small as possible.
- Make the reason and required invariants clear near each unsafe operation.
- Check relevant FFI return values and propagate operating-system errors.
- Preserve stdin forwarding, process-group handling, signal forwarding, and exit status.
- Do not leak command arguments or secret values into normal output or summaries.

## Resources and Logs

- Treat memory, file descriptors, child processes, and temporary files as bounded resources.
- Avoid unnecessary full-log copies or other allocations proportional to input size.
- Preserve cleanup on success, error, and signal paths.
- Keep log-directory ownership and permission checks at least as strict as they are now.
- Keep retained log files private and their retention bounded.
- Keep summaries deterministic, concise, and limited by configured bounds.
- Do not print a full log when a bounded summary is expected.

## Tests

- Add or update tests for behavior changes.
- Include failure paths when changing process, signal, logging, or parsing code.
- Keep testability in mind without adding production abstractions solely for tests.
