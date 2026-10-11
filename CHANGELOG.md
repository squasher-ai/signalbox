# Changelog

All notable changes to Squasher Signalbox are documented here.

## [Unreleased]

- Added `service_version` and a `releases` schedule (`--service-version`, `--release VERSION@SECONDS`). The default `service.version` is now `1.0.0`; the generator version stays in `squasher.generator.version`.
- Added fault scenarios (`faults`, `--fault`): latency regression tied to a release, error spike, database saturation, upstream outage, log burst, and GenAI rate limit, context length, refusal, and tool error. Each run writes a deterministic `ground_truth.json`.
- Added the `web-topology` preset: gateway and API services with database and cache clients in one trace.
- Added `run_id` (`--run-id`) to change trace, span, and conversation IDs without changing values. Trace IDs now depend on a seed-derived salt by default.
- Added `--send` for direct OTLP/HTTP export with gzip, retries that honor `Retry-After`, bounded concurrency, and headers from `SIGNALBOX_OTLP_HEADERS`.
- GenAI model spans now carry `gen_ai.response.finish_reasons`; web duration metrics use the request duration.

- Rename the repository, crate, and CLI to `signalbox`.
- Require Rust 1.88 to match locked AWS SDK dependencies.

- Renamed the project to Squasher Signalbox and added the branded `signalbox` CLI.
- Added shell completion generation for Bash, Elvish, Fish, PowerShell, and Zsh.
- Added agent, RAG, and web presets with correlated traces, logs, and metrics.
- Added local and S3-compatible create-only sinks, crash-safe local writes, JSON plans, schemas, manifests, and progress output.
- Added synthetic resource provenance and current GenAI token modality attributes.
- Added a reproducible local release benchmark and a comparison with related generators.
