# Faults and ground truth

Faults inject time-bounded anomalies into a run. Each run writes
`ground_truth.json` next to `manifest.json`, so a detector, an agent, or a
dashboard check can be scored against known answers.

## Quick start

```sh
# Named presets. Windows scale with the run length.
signalbox generate --preset web-topology --count 3600 \
  --fault release-regression,db-saturation --output ./incident

# Check the windows first.
signalbox generate --dry-run --preset agent --fault genai-rate-limit,genai-refusal \
  --output ./preview
```

`signalbox presets` lists the fault presets, their kinds, default rates, and the
scenario presets that support them.

## Fault kinds

| Kind | Presets | Effect | Labels |
| --- | --- | --- | --- |
| `latency_regression` | all | The work span (model call or database query) is 6x longer; the root and later spans move with it | `span_duration_increase`, `duration_histogram_shift`, `version_change` |
| `error_spike` | all | The request fails with `error.type=UnhandledException` (HTTP 500 for web) | `error_rate_increase`, `error_logs` |
| `db_saturation` | `web`, `web-topology` | The query takes 1.5-3 s longer; one in four times out (`error.type=timeout`) | `dependency_latency_increase`, `dependency_timeouts` |
| `upstream_outage` | `agent`, `rag`, `web-topology` | The model provider, or the API behind the gateway, returns 503 | `upstream_5xx`, `error_rate_increase` |
| `log_burst` | all | 20 extra WARN logs per scenario, correlated to the trace | `log_volume_increase`, `warn_logs` |
| `genai_rate_limit` | `agent`, `rag` | The model call returns 429 (`rate_limit_exceeded`) | `provider_rate_limit` |
| `genai_context_length` | `agent`, `rag` | Input tokens exceed 128k; the call returns 400 (`context_length_exceeded`) | `input_tokens_exceed_context` |
| `genai_refusal` | `agent`, `rag` | `gen_ai.response.finish_reasons=["content_filter"]`, few output tokens | `finish_reason_content_filter`, `low_output_tokens` |
| `genai_tool_error` | `agent`, `rag` | The tool span fails (`error.type=tool_error`); the agent completes | `tool_span_errors` |

## Configuration

```json
{
  "faults": [
    { "kind": "error_spike", "start_s": 600, "end_s": 900, "rate": 50 },
    { "kind": "latency_regression", "start_s": 1800, "end_s": 3600, "version": "2.4.0" }
  ]
}
```

- `start_s` and `end_s` are seconds from `start_ns`. A scenario is in the window
  when its start time is in `[start_s, end_s)`. The window must start before the
  end of the run.
- `rate` is the percent of scenarios in the window that are affected (1..=100).
  The default depends on the kind. `log_burst` always affects every scenario in
  its window.
- `version` limits the fault to scenarios served by that version. It must be
  `service_version` or a release version.

Faults other than `log_burst` are exclusive. For each scenario, the first listed
fault whose window, version, and rate draw match is applied, and no later
fault. `log_burst` adds logs on top of any other fault.

## Presets

`--fault` replaces the configured `faults` with explicit entries. The presets
other than `release-regression` share the span from 40% to 90% of the run in
equal, consecutive windows, in the order given. This leaves a baseline before
and a recovery after each fault. `release-regression` adds a release at 50% of
the run (`1.0.0` becomes `1.1.0`; other version strings get `-next`) and a
`latency_regression` limited to that version until the end of the run. When it
is combined with other presets, they share 10% to 45% instead, so no window
overlaps the release. The manifest records the expanded configuration.

## `ground_truth.json`

The file is deterministic: it has no elapsed time or host data. Example (some
fields omitted):

```json
{
  "schema_version": 1,
  "preset": "web-topology",
  "run_id": null,
  "id_salt": "0d3c8d6b6f0e4a5b",
  "service": "synthetic-api",
  "versions": [
    { "version": "1.0.0", "records": { "first": 0, "end_exclusive": 1800 } },
    { "version": "1.1.0", "records": { "first": 1800, "end_exclusive": 3600 } }
  ],
  "faults": [
    {
      "id": "fault-1",
      "kind": "latency_regression",
      "start_time_unix_nano": 1759192200000000000,
      "end_time_unix_nano": 1759194001000000000,
      "records": { "first": 1800, "end_exclusive": 3600 },
      "rate": 100,
      "affected_records": 1800,
      "service": "synthetic-api",
      "service_versions": ["1.1.0"],
      "span_names": ["SELECT orders", "GET /orders"],
      "dependency": null,
      "error_type": null,
      "http_status_code": null,
      "finish_reason": null,
      "expected_signals": ["traces", "metrics"],
      "indicators": ["span_duration_increase", "duration_histogram_shift", "version_change"]
    }
  ]
}
```

| Field | Meaning |
| --- | --- |
| `records` | Scenario indices in the window. Scenario `i` starts at `start_ns + i * interval_ns`. |
| `affected_records` | Exact number of scenarios that carry this fault. |
| `extra_logs` | Extra log records added by `log_burst`. |
| `service`, `service_versions` | The service with the fault and the versions active in the window (or the filtered version). |
| `span_names` | Spans that show the fault, most specific first. |
| `dependency` | The dependency at fault (`postgresql`, `openai`, or the upstream service), when there is one. |
| `error_type`, `http_status_code`, `finish_reason` | Values to expect on affected spans and metric points. |
| `expected_signals`, `indicators` | Where the fault is visible and how it shows. |

Baseline failures from `error_rate` continue during a run and use
`error.type=SyntheticError`. They are not labeled as faults; set `error_rate` to
`0` for a clean baseline.

## Scoring suggestions

- **Detection:** an alert or finding counts when it overlaps the fault window
  and names the fault's service.
- **Diagnosis:** compare the named version, span, dependency, and error type
  with the label.
- **False positives:** run the same configuration without faults, or score
  findings outside every window.
