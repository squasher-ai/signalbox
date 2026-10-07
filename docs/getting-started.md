# Getting started

## Build

Install stable Rust with [rustup](https://rustup.rs/), clone the repository, and build a release binary:

```sh
git clone https://github.com/squasher-ai/otel-agent-forge.git
cd otel-agent-forge
cargo build --release
./target/release/otel-agent-forge --help
```

## Generate a fixture

Start with a small JSON fixture while checking the output shape:

```sh
./target/release/otel-agent-forge generate \
  --count 25 \
  --batch-size 10 \
  --format json \
  --output ./fixture
```

The command prints one summary JSON value on stdout. The progress bar, when enabled, is on stderr. This lets a caller capture stdout as a value without parsing terminal output.

## Replay through a Collector

Point an OpenTelemetry Collector `filelog`/file receiver or a small test harness at the generated files. Protobuf files contain `Export*ServiceRequest` messages; JSON files use OTLP JSON field names and represent 64-bit integers as decimal strings. The request type is selected by the file prefix (`traces`, `logs`, or `metrics`).

## Reproduce a run

Save the command or a JSON config file with the manifest. The manifest includes the effective configuration, schema provenance, shard pattern, and counts. Reusing the same configuration and seed produces the same bytes, regardless of `--jobs`.
