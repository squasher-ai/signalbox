# Configuration

Every `generate` flag can be represented in JSON. Generate the exact schema and defaults for the installed binary:

```sh
otel-agent-forge schema > config-schema.json
```

A config file is read with `--config path.json` or from stdin with `--config -`. CLI flags override file values. Unknown fields are rejected. The main limits are:

| Field | Default | Constraint |
| --- | --- | --- |
| `count` | `10000` | positive |
| `batch_size` | `1000` | `1..=100000` |
| `jobs` | up to 8 | `1..=256`, bounded in-flight work |
| `seed` | `42` | unsigned 64-bit |
| `preset` | `agent` | `agent`, `rag`, `web` |
| `signals` | all three | unique, non-empty |
| `format` | `protobuf` | `protobuf` or `json` |
| `error_rate` | `5` | `0..=100` |

The generated manifest records the effective configuration. Keep that manifest with a fixture when it is used as a test artifact.
