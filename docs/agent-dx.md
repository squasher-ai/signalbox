# Agent and automation DX

The CLI is designed to be called by an agent or a CI process without scraping human output.

## Stream contract

- **stdout:** exactly one JSON value on success (a plan for `--dry-run`, otherwise a summary).
- **stderr:** progress only; disable it with `--progress never`.
- **exit status:** zero on success; non-zero on validation, I/O, storage, or interruption errors.
- **error value:** `{ "schema_version": 1, "status": "error", "error": { "code": "...", "message": "..." } }`.

Messages avoid credentials, request bodies, and endpoint query strings. Treat `code` as the stable field for branching; display `message` to a human.

## Safe planning

Call `generate --dry-run` first. It validates the complete configuration, reports counts and shard names, and does not create a directory or contact S3. `schema` provides a machine-readable JSON Schema and defaults; `presets` reports per-scenario cardinality.

## Deterministic retries

Set an explicit `--seed`, `--batch-size`, `--jobs`, and `--start-ns` in automation. A retry must use a new output target because successful files are create-only. The same effective config and seed produce identical bytes even if `--jobs` changes.
