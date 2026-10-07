# Contributing

Thanks for helping improve OTEL Agent Forge. Small, focused pull requests are easiest to review.

## Before you start

- Search existing issues and discussions before opening a new one.
- Do not include real customer telemetry, prompts, secrets, or credentials in fixtures or logs.
- Keep the CLI's stdout, stderr, exit status, and create-only output guarantees stable unless the change is intentional and documented.
- For semantic-convention changes, include the upstream registry URL, schema version, commit, and a test that covers the changed output.

## Local checks

Run these from the repository root:

```sh
cargo fmt --all -- --check
cargo check --locked
cargo test --locked
cargo clippy --all-targets --all-features -- -D warnings
cargo doc --no-deps --locked
```

Add a test for every externally observable behavior change. For performance work, record the exact command, signal set, format, Rust version, CPU, and memory. Avoid adding a benchmark that only mirrors an implementation detail.

## Pull requests

Describe the user-visible behavior, the compatibility impact, and the checks you ran. Keep generated output and dependency updates in separate commits when practical. A maintainer will run the full CI workflow before merge.

## Commit and release notes

Use clear imperative commit subjects. Update `CHANGELOG.md` for user-visible changes. Release artifacts are built by maintainers from a tagged commit; contributors should not publish crates or binaries from personal accounts.
