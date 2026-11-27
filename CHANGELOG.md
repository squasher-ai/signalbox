# Changelog

All notable changes to Squasher Signalbox are documented here.

## [Unreleased] - 0.2.0

### Added

- Added `service_version` and a `releases` schedule (`--service-version`, `--release VERSION@SECONDS`). The default `service.version` is now `1.0.0`; the generator version stays in `squasher.generator.version`.
- Added fault scenarios (`faults`, `--fault`): latency regression tied to a release, error spike, database saturation, upstream outage, log burst, and GenAI rate limit, context length, refusal, and tool error. Each run writes a deterministic `ground_truth.json`.
- Added the `web-topology` preset: gateway and API services with database and cache clients in one trace.
- Added `run_id` (`--run-id`) to change trace, span, and conversation IDs without changing values. Trace IDs now depend on a seed-derived salt by default.
- Added `--send` for direct OTLP/HTTP export with gzip, retries that honor `Retry-After`, bounded concurrency, and headers from `SIGNALBOX_OTLP_HEADERS`.
- GenAI model spans now carry `gen_ai.response.finish_reasons`; web duration metrics use the request duration.

### Changed

- Fixtures from 0.1.0 are not byte-identical to 0.2.0 output for the same seed and configuration. Regenerate stored fixtures and golden files, and update tests that match exact IDs or versions:
  - The default `service.version` is now `1.0.0`. It was the generator version.
  - Trace, span, and conversation IDs now include a seed-derived salt, so every ID changes. Use `--run-id` to change IDs between repetitions without changing values.
  - GenAI model spans add `gen_ai.response.finish_reasons`, and web duration metrics use the request duration.
- In GenAI presets, `error_spike` metric points now carry `error.type`, as `ground_truth.json` states.
- A `release-regression` fault preset with an existing release of the same version now starts the regression at that release's offset, not at 50% of the run.

### Security

- `--send` ignores `HTTP_PROXY`, `HTTPS_PROXY`, and `ALL_PROXY`, so payloads and header values go only to the configured endpoint.
- `--send` reads at most 64 KiB of a success response body and fails the run on a larger body. A truncated or unreadable success body also fails the send.

### Fixed

- `--send` accepts `Retry-After` as an HTTP date as well as delay seconds, capped at 60 s.

## [0.1.0]

- Rename the repository, crate, and CLI to `signalbox`.
- Require Rust 1.88 to match locked AWS SDK dependencies.

- Renamed the project to Squasher Signalbox and added the branded `signalbox` CLI.
- Added shell completion generation for Bash, Elvish, Fish, PowerShell, and Zsh.
- Added agent, RAG, and web presets with correlated traces, logs, and metrics.
- Added local and S3-compatible create-only sinks, crash-safe local writes, JSON plans, schemas, manifests, and progress output.
- Added synthetic resource provenance and current GenAI token modality attributes.
- Added a reproducible local release benchmark and a comparison with related generators.
