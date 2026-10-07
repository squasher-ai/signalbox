# OTEL Agent Forge

[![CI](https://github.com/squasher-ai/otel-agent-forge/actions/workflows/ci.yml/badge.svg)](https://github.com/squasher-ai/otel-agent-forge/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Fast, deterministic synthetic [OpenTelemetry](https://opentelemetry.io/) traces, logs, and metrics for AI-agent tests, collector fixtures, demos, and load checks.

Agent Forge creates correlated OTLP export requests. A single scenario has one trace, related logs, and related metric points, so a test can replay a realistic agent, RAG, or web request without maintaining hand-written fixtures.

## Install

Build from source with a recent stable Rust toolchain:

```sh
cargo install --path .
otel-agent-forge --help
```

You can also run it without installing:

```sh
cargo run --release -- generate --count 10_000 --output ./fixture
```

## Quick start

```sh
# Protobuf (the default), three signals, 1,000 scenarios per shard.
otel-agent-forge generate --count 10_000 --output ./agent-fixture

# OTLP/JSON for a human-readable fixture.
otel-agent-forge generate --preset rag --format json --output ./rag-fixture

# Validate a plan without creating files or contacting S3.
otel-agent-forge generate --dry-run --count 100 --output ./preview

# Inspect machine-readable defaults and presets.
otel-agent-forge schema > config-schema.json
otel-agent-forge presets
```

Each output target must be new. Local output uses create-only files; S3 output uses conditional writes. A run writes one file per signal and shard plus `manifest.json`:

```
traces-000000000000.otlp.pb
logs-000000000000.otlp.pb
metrics-000000000000.otlp.pb
manifest.json
```

Progress is written to stderr. Stdout contains exactly one JSON plan or summary, which makes the command safe to call from an agent, CI job, or shell pipeline. Use `--progress never` for fully quiet automation. Errors have stable `code` and `message` fields and a non-zero exit status.

## S3 and compatible storage

Use the AWS default credential chain and an `s3://bucket/prefix` target:

```sh
AWS_PROFILE=fixtures otel-agent-forge generate \
  --output s3://my-bucket/agent-runs/run-001 \
  --region us-east-1
```

For MinIO, LocalStack, or another S3-compatible service, pass a credential-free endpoint and explicit static credentials through environment variables. Agent Forge never forwards the default AWS profile or instance credentials to a custom endpoint. It forces path-style addressing and keeps the same create-only guarantee:

```sh
AWS_ACCESS_KEY_ID=minioadmin AWS_SECRET_ACCESS_KEY=minioadmin \
  otel-agent-forge generate \
  --output s3://fixtures/agent-runs/run-001 \
  --endpoint http://127.0.0.1:9000 \
  --region us-east-1
```

See [docs/s3-compatible.md](docs/s3-compatible.md) for provider setup and required permissions.

## Design

- **Correlated signals:** trace IDs, span IDs, log IDs, conversation IDs, timestamps, and failures line up across signals.
- **Deterministic:** the same seed and configuration produce byte-identical shards, independent of worker count.
- **Bounded memory:** workers generate one shard at a time; `jobs * batch_size` is capped at one million scenarios.
- **Fast:** generation is parallel across shards and encoding happens off the async runtime. A release build is the right choice for load fixtures.
- **Safe writes:** output directories must not exist, files use create-only semantics, and S3 uses conditional puts.
- **Agent-friendly:** progress and results use separate streams, plans are JSON, schemas are discoverable, and secrets are never included in errors or manifests.

Read [docs/architecture.md](docs/architecture.md) for the execution model and [docs/agent-dx.md](docs/agent-dx.md) for automation contracts.

## OpenTelemetry compatibility

The generator emits OTLP protobuf or OTLP/JSON export requests accepted by the OpenTelemetry Collector. Core attributes use the pinned OpenTelemetry schema URL. Agent and RAG scenarios use the GenAI development schema URL and explicitly identify their pinned upstream commit. GenAI metric names are development conventions and may change upstream; they are not presented as stable API.

The key snapshot and provenance are in [`src/semconv.rs`](src/semconv.rs). See [docs/standards.md](docs/standards.md) for the conventions and links to the upstream registry and [semconv.com](https://semconv.com/).

## Development

```sh
cargo fmt --all -- --check
cargo check --locked
cargo test --locked
cargo clippy --all-targets --all-features -- -D warnings
cargo doc --no-deps --locked
```

Use `cargo run --release -- generate --count 100_000 --output ./bench-fixture --progress never` when comparing throughput. Include the command, hardware, Rust version, format, and signal set with benchmark results.

## License

MIT. See [LICENSE](LICENSE).
