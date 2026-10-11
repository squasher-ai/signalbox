# Configuration

Every `generate` flag can be represented in JSON. Generate the exact schema and defaults for the installed binary:

```sh
signalbox schema > config-schema.json
```

A config file is read with `--config path.json` or from stdin with `--config -`. CLI flags override file values. Unknown fields are rejected. The main limits are:

| Field | Default | Constraint |
| --- | --- | --- |
| `count` | `10000` | positive |
| `batch_size` | `1000` | `1..=100000` |
| `jobs` | up to 8 | `1..=256`, bounded in-flight work |
| `seed` | `42` | unsigned 64-bit |
| `preset` | `agent` | `agent`, `rag`, `web`, `web-topology` |
| `signals` | all three | unique, non-empty |
| `format` | `protobuf` | `protobuf` or `json` |
| `error_rate` | `5` | `0..=100` |
| `service_version` | `1.0.0` | 1..=256 characters |
| `releases` | `[]` | up to 64; strictly increasing `at_s`; new versions; before the end of the run |
| `faults` | `[]` | up to 64; see [faults.md](faults.md) |
| `run_id` | derived from `seed` | 1..=256 characters; changes IDs only |
| `send` | none | HTTP(S) base URL without credentials, query, or fragment |

`--release VERSION@SECONDS`, `--fault NAME`, `--run-id`, and `--send` set the same fields. `--release` and `--fault` replace the file values; `--fault` presets are expanded to explicit `faults` (and, for `release-regression`, a release), so the manifest shows exact windows.

Example:

```json
{
  "preset": "web-topology",
  "count": 3600,
  "service_version": "2.3.0",
  "releases": [{ "at_s": 1800, "version": "2.4.0" }],
  "faults": [
    { "kind": "latency_regression", "start_s": 1800, "end_s": 3600, "version": "2.4.0" },
    { "kind": "db_saturation", "start_s": 600, "end_s": 900, "rate": 80 }
  ],
  "run_id": "nightly-2026-10-11"
}
```

Releases apply to the primary service: the single service of `agent`, `rag`, and `web`, or `synthetic-api` in `web-topology`. In `web-topology`, `service_name` is not used; the services are `synthetic-gateway` and `synthetic-api`.

`send` headers are read only from the `SIGNALBOX_OTLP_HEADERS` environment variable. They are not part of the configuration and never appear in plans, manifests, or errors.

The generated manifest records the effective configuration. Keep that manifest with a fixture when it is used as a test artifact.
