# Architecture

Squasher Signalbox is a single binary with a small pipeline:

1. **Resolve and validate.** CLI flags override a JSON config. Unknown config fields, unsafe S3 targets, invalid ranges, and credential-bearing endpoints fail before output is created.
2. **Reserve the target.** A local target is created with `create_dir`; an S3 target is checked and written with conditional requests. Existing runs are never overwritten.
3. **Schedule shards.** An atomic cursor assigns each shard to one of up to `jobs` workers. A shard contains at most `batch_size` scenarios.
4. **Generate and encode.** Each signal is generated from the scenario index and seed. Encoding runs in blocking tasks so CPU work does not stall the Tokio runtime.
5. **Write and summarize.** Workers write immutable shard objects, while one progress bar tracks scenarios. The final manifest is written after all signal shards succeed.

Squasher Signalbox does not retain all scenarios in memory. Peak scenario state is bounded by the configured in-flight shard limit (`jobs * batch_size`, capped at one million). The output order is independent of scheduling because every ID and timestamp is derived from `(seed, scenario index, lane)`.

A failed run leaves its partial files in place. Retry with a new target; this prevents an interrupted run from being mistaken for a complete fixture. Ctrl-C returns exit code 130 and emits a stable `interrupted` error.

## Data model

An agent scenario contains a root operation, model/inference work, and a tool call. RAG adds retrieval. A web scenario uses an HTTP root and a database child. All scenarios carry the same resource and instrumentation scope across signals. Failed scenarios set status/error attributes consistently and use the configured error rate.

## Extension points

- Add a preset in `src/config.rs` and its signal builders.
- Add attributes in `src/semconv.rs` with an upstream source and schema version.
- Keep output contracts and tests in `src/tests.rs`; externally visible changes should update the README and docs.
