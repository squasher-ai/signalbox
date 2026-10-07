# Squasher Signalbox docs

Start with a small, inspectable fixture. Scale the same scenario after the output contract is clear.

| Guide | Use it to |
| --- | --- |
| [Getting started](getting-started.md) | Build the CLI, generate a fixture, and send it to a local Collector |
| [Configuration](configuration.md) | Set counts, formats, scenarios, and deterministic time |
| [Agents and automation](agent-dx.md) | Call the CLI with JSON input and parseable results |
| [S3-compatible storage](s3-compatible.md) | Store shards in AWS S3, MinIO, or a compatible service |
| [Standards and provenance](standards.md) | Inspect the OTLP wire format and semantic-convention pins |
| [Architecture](architecture.md) | Understand the bounded generation and write pipeline |
| [Performance](performance.md) | Run the reproducible local benchmark and compare changes |
| [Other generators](alternatives.md) | Select the right tool and see the comparison |

The root [README](../README.md) contains the quick start. The installed binary's `schema` command is the source for configuration fields and defaults.
