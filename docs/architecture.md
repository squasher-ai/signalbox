# Architecture

Squasher Signalbox is a single binary with a small pipeline:

1. **Resolve and validate.** CLI flags override a JSON config. Unknown config fields, unsafe S3 targets, invalid ranges, and credential-bearing endpoints fail before output is created.
2. **Reserve the target.** A local target is created with `create_dir`; an S3 target is checked and written with conditional requests. Existing runs are never overwritten.
3. **Schedule shards.** An atomic cursor assigns each shard to one of up to `jobs` workers. A shard contains at most `batch_size` scenarios.
4. **Generate and encode.** Each signal is generated from the scenario index and seed. Encoding runs in blocking tasks so CPU work does not stall the Tokio runtime.
5. **Write or send, then summarize.** Workers write immutable shard objects, or post them to an OTLP/HTTP endpoint with `--send`, while one progress bar tracks scenarios. Workers also count the scenarios each fault affected. `ground_truth.json` and then `manifest.json` are written after all signal shards succeed.

Squasher Signalbox does not retain all scenarios in memory. Peak scenario state is bounded by the configured in-flight shard limit (`jobs * batch_size`, capped at one million). The output order is independent of scheduling because every ID and timestamp is derived from `(seed, scenario index, lane)`.

A failed run leaves its partial files in place. Retry with a new target; this prevents an interrupted run from being mistaken for a complete fixture. Ctrl-C returns exit code 130 and emits a stable `interrupted` error.

## Data model

An agent scenario contains a root operation, model/inference work, and a tool call. RAG adds retrieval. A web scenario uses an HTTP root and a database child. A `web-topology` scenario crosses two services: a gateway server and client span, then an API server span with cache and database client spans. Spans, logs, and metric points are grouped by resource (service and version), so a shard can contain several resources. Failed scenarios set status/error attributes consistently and use the configured error rate.

Releases and faults are resolved per scenario from the seed and index. Faults other than `log_burst` are exclusive: the first listed fault whose window, version, and rate draw match a scenario applies, so each scenario has at most one label. The run ID (or a seed-derived salt) is mixed into IDs only, through a bijective mixer, so two run IDs cannot produce the same trace ID for the same seed.

## Extension points

- Add a preset in `src/config.rs` and its signal builders.
- Add a fault kind in `src/faults.rs`, its effect in `src/model.rs`, and its label in `src/ground_truth.rs`.
- Add attributes in `src/semconv.rs` with an upstream source and schema version.
- Keep output contracts and tests in `src/tests.rs`; externally visible changes should update the README and docs.
