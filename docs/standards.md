# Standards and provenance

Squasher Signalbox follows the OpenTelemetry Protocol (OTLP) export request shapes and the semantic-conventions registry. It emits the current key names from a small pinned snapshot in [`src/semconv.rs`](../src/semconv.rs), rather than inventing vendor-specific aliases.

## Schema URLs

- Core signals use `https://opentelemetry.io/schemas/1.44.0`.
- Agent and RAG signals use the GenAI development schema URL `https://opentelemetry.io/schemas/gen-ai-dev/1.42.0-dev`.
- The GenAI attribute snapshot records its upstream commit in the manifest.

GenAI conventions and metrics are still development material. Consumers should tolerate additions, renames, and changes in stability. The fixture generator makes the selected versions visible so a test can pin or compare them.

The manifest records two GenAI pins: the attribute snapshot commit and the
newer metric-definition commit. They are separate because upstream metric
names changed after the attribute snapshot was pinned. Token metrics carry
`gen_ai.token.modality` with the value `text`.

Every resource also carries the Squasher extension attributes
`squasher.synthetic=true`, `squasher.generator.name`, and
`squasher.generator.version`. Use them to filter generated traffic in a test
backend without confusing it with application telemetry.

## Naming sources

Use the official OpenTelemetry semantic-conventions registry and its generated documentation first:

- [OpenTelemetry semantic conventions](https://opentelemetry.io/docs/specs/semconv/)
- [OpenTelemetry GenAI conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/)
- [semconv.com](https://semconv.com/) for browsing names and stability status
- [OTLP specification](https://opentelemetry.io/docs/specs/otlp/)

The local snapshot intentionally contains only attributes needed by the fixture model. It does not copy Squasher ingest normalization rules. When a convention changes, update the key, schema URL, and provenance together, then add or update a fixture assertion.

## Wire formats

Protobuf output is the encoded `ExportTraceServiceRequest`, `ExportLogsServiceRequest`, or `ExportMetricsServiceRequest`. JSON output follows OTLP JSON field casing; 64-bit integer fields are decimal strings as required by the mapping. Both forms are designed for Collector receivers and test harnesses, not for direct ingestion into a vendor-specific endpoint.
