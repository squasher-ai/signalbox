# Agents and automation

The CLI is designed to be called by an agent or a CI process without scraping human output.

## Stream contract

- **stdout:** exactly one JSON value on normal command success (a plan for `--dry-run`, otherwise a summary). `--help`, `--version`, and `completions` are intentional discovery exceptions and write human-readable text or a shell script.
- **stderr:** progress and a structured error on failure; disable progress with `--progress never`.
- **exit status:** zero on success; non-zero on validation, I/O, storage, or interruption errors.
- **error value:** `{ "schema_version": 1, "status": "error", "error": { "code": "...", "message": "..." } }`.

Messages avoid credentials, request bodies, and endpoint query strings. Treat `code` as the stable field for branching; display `message` to a human.

## Safe planning

Call `generate --dry-run` first. It validates the complete configuration, reports counts and shard names, and does not create a directory or contact S3. `schema` provides a machine-readable JSON Schema and defaults; `presets` reports per-scenario cardinality.

## Deterministic retries

Set an explicit `--seed`, `--batch-size`, `--jobs`, and `--start-ns` in automation. A retry must use a new output target because successful files are create-only. Treat a run as complete only when `manifest.json` exists. The same effective config and seed produce identical bytes even if `--jobs` changes.

## Minimal workflow

1. Read `schema` and `presets` to select a supported configuration.
2. Call `generate --dry-run` and check the scenario and signal counts.
3. Generate into a new target with an explicit seed and timestamp.
4. Keep the summary, manifest, and effective configuration with the test result.

A dry run validates configuration; it does not check a bucket, credentials, disk space, or endpoint reachability. Those are checked during generation.

## Shell completion

Generate a script from the same binary that will run it, so completion stays
aligned with the installed flags:

```sh
eval "$(signalbox completions zsh)"
```

The command supports `bash`, `elvish`, `fish`, `powershell`, and `zsh`. Keep
completion output separate from JSON workflows; it is intended for shell
startup files and interactive use.
