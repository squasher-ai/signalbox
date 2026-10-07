# How Squasher Signalbox fits

Squasher Signalbox sits between an endpoint load generator and a hand-written fixture. It produces complete, offline OTLP export requests for a connected scenario, then stores those requests as local files or objects in S3-compatible storage.

The comparison below records the roles of the tools reviewed for this project. Their interfaces and feature sets can change; check each project's current documentation before selecting one.

| Tool | Best at | Signalbox's difference |
| --- | --- | --- |
| [Collector `telemetrygen`](https://github.com/open-telemetry/opentelemetry-collector-contrib/tree/main/cmd/telemetrygen) | Sending trace, log, and metric load to an OTLP endpoint | Offline fixtures, agent/RAG/web scenario shapes, deterministic shards, and S3 output |
| [OTelForge](https://github.com/ChubV/otelforge) | Agent-oriented mock conversations, streaming, RAG, and tool loops | Rust CLI, all-signal correlation, OTLP protobuf/JSON files, create-only sinks, and machine-readable plans |
| [`otelgen`](https://github.com/krzko/otelgen) | Broad signal primitives, events, links, and exemplars | Opinionated connected presets and durable local/object-store fixtures |
| [motel](https://github.com/andrewh/motel) | YAML service topologies with correlated generic telemetry | Agent and retrieval stories plus a stable OTLP archive contract |
| [OTel Arrow traffic generator](https://github.com/open-telemetry/otel-arrow/tree/main/rust/otap-dataflow/crates/dev-nodes/src/receivers/traffic_generator) | High-throughput traffic inside an experimental Rust dataflow | A user-facing fixture CLI with agent semantics and reproducible files |

Use `telemetrygen` when the question is “can this collector handle a stream?” Use an agent-focused simulator when the question is “does this application handle a conversation?” Use Signalbox when the test needs a repeatable, inspectable fixture that explains one scenario across traces, logs, and metrics.

## Shared conventions

Signalbox follows the conventions that make fixtures useful across tools:

- OTLP export request envelopes, in protobuf or OTLP JSON.
- W3C-compatible trace and span identifiers with valid parent/child timing.
- OpenTelemetry semantic-convention attributes with a recorded schema URL and source commit.
- GenAI attributes marked as development material until the upstream registry stabilizes them.
- Metadata, IDs, timing, and counts; the presets do not copy real prompts or tool content.
- JSON plans, manifests, and stable error codes for automation.

See [standards.md](standards.md) for the pinned registry snapshot and [agent-dx.md](agent-dx.md) for the CLI stream contract.
