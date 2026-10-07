# Getting started

## Build

Install stable Rust with [rustup](https://rustup.rs/), clone the repository, and build a release binary:

```sh
git clone https://github.com/squasher-ai/signalbox.git
cd signalbox
cargo build --release
./target/release/signalbox --help
```

## Generate a fixture

Start with a small JSON fixture while checking the output shape:

```sh
./target/release/signalbox generate \
  --count 25 \
  --batch-size 10 \
  --format json \
  --output ./fixture
```

The command prints one summary JSON value on stdout. The progress bar, when enabled, is on stderr. This lets a caller capture stdout as a value without parsing terminal output.

## Replay through a Collector

Use a Collector with its OTLP HTTP receiver enabled on `127.0.0.1:4318`. Each JSON shard is one complete export request. Send it to the matching signal endpoint:

```sh
for signal in traces logs metrics; do
  for file in ./fixture/"$signal"-*.otlp.json; do
    [ -f "$file" ] || continue
    curl --fail --show-error --silent \
      -H 'Content-Type: application/json' \
      --data-binary @"$file" \
      "http://127.0.0.1:4318/v1/$signal"
  done
done
```

Protobuf shards contain raw `ExportTraceServiceRequest`, `ExportLogsServiceRequest`, or `ExportMetricsServiceRequest` messages. Send these to the same endpoints with `Content-Type: application/x-protobuf`. The file prefix selects the request type. The payloads have no length prefix or JSONL framing; do not treat them as arbitrary log lines.

## Reproduce a run

Save the command or a JSON config file with the manifest. The manifest includes the effective configuration, schema provenance, shard pattern, and counts. Reusing the same configuration and seed produces the same bytes, regardless of `--jobs`.
