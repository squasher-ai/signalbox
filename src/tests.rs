use crate::semconv::attributes as attr;
use crate::{
    cli::{Cli, Command},
    config::{Config, Format, Preset, Progress},
    logs, metrics, runner, traces,
};
use clap::Parser;
use opentelemetry_proto::tonic::{
    collector::{
        logs::v1::ExportLogsServiceRequest, metrics::v1::ExportMetricsServiceRequest,
        trace::v1::ExportTraceServiceRequest,
    },
    common::v1::any_value::Value,
    metrics::v1::{AggregationTemporality, metric::Data},
};
use prost::Message;

#[tokio::test]
async fn parallel_output_preserves_determinism_shards_and_signal_correlation() {
    let temporary = tempfile::tempdir().unwrap();
    let mut runs = Vec::new();
    for jobs in [1, 3] {
        let output = temporary.path().join(format!("run-{jobs}"));
        let config = Config {
            count: 5,
            batch_size: 2,
            jobs,
            preset: Preset::Rag,
            format: Format::Protobuf,
            output: output.to_str().unwrap().into(),
            progress: Progress::Never,
            error_rate: 100,
            ..Config::default()
        };
        let summary = runner::run(config).await.unwrap();
        assert_eq!(
            (summary.records, summary.spans, summary.logs, summary.metric_points),
            (5, 20, 5, 25)
        );
        assert_eq!(summary.shards_per_signal, 3);
        assert!(output.join("manifest.json").exists());
        runs.push(output);
    }
    let mut trace_count = 0;
    for shard in 0..3 {
        for signal in ["traces", "logs", "metrics"] {
            let file = format!("{signal}-{shard:012}.otlp.pb");
            assert_eq!(
                std::fs::read(runs[0].join(&file)).unwrap(),
                std::fs::read(runs[1].join(&file)).unwrap()
            );
        }
        let traces = ExportTraceServiceRequest::decode(
            std::fs::read(runs[0].join(format!("traces-{shard:012}.otlp.pb"))).unwrap().as_slice(),
        )
        .unwrap();
        let logs = ExportLogsServiceRequest::decode(
            std::fs::read(runs[0].join(format!("logs-{shard:012}.otlp.pb"))).unwrap().as_slice(),
        )
        .unwrap();
        let metrics = ExportMetricsServiceRequest::decode(
            std::fs::read(runs[0].join(format!("metrics-{shard:012}.otlp.pb"))).unwrap().as_slice(),
        )
        .unwrap();
        let spans = &traces.resource_spans[0].scope_spans[0].spans;
        let resource = traces.resource_spans[0].resource.as_ref().unwrap();
        assert_eq!(Some(resource), logs.resource_logs[0].resource.as_ref());
        assert_eq!(Some(resource), metrics.resource_metrics[0].resource.as_ref());
        for (key, value) in [
            (attr::SQUASHER_SYNTHETIC, Value::BoolValue(true)),
            (attr::SQUASHER_GENERATOR_NAME, Value::StringValue(env!("CARGO_PKG_NAME").into())),
            (
                attr::SQUASHER_GENERATOR_VERSION,
                Value::StringValue(env!("CARGO_PKG_VERSION").into()),
            ),
        ] {
            assert!(resource.attributes.iter().any(|attribute| {
                attribute.key == key
                    && attribute.value.as_ref().and_then(|value| value.value.as_ref())
                        == Some(&value)
            }));
        }
        trace_count += spans.len();
        for (root, log) in
            spans.chunks_exact(4).zip(&logs.resource_logs[0].scope_logs[0].log_records)
        {
            assert_eq!(root[0].trace_id, log.trace_id);
            assert_eq!(root[0].span_id, log.span_id);
            assert_eq!(log.severity_number, 17);
            assert!(root[0].parent_span_id.is_empty());
            for child in &root[1..] {
                assert_eq!(child.parent_span_id, root[0].span_id);
                assert_eq!(child.trace_id.len(), 16);
                assert_ne!(child.span_id, root[0].span_id);
                assert!(child.start_time_unix_nano >= root[0].start_time_unix_nano);
                assert!(child.end_time_unix_nano <= root[0].end_time_unix_nano);
            }
        }
    }
    assert_eq!(trace_count, 20);
}

#[test]
fn generated_metrics_have_valid_buckets_delta_windows_and_current_attributes() {
    for preset in [Preset::Agent, Preset::Rag, Preset::Web] {
        let config = Config { preset, count: 20, error_rate: 0, ..Config::default() };
        let request = metrics::generate(&config, 0, config.count);
        let resource = &request.resource_metrics[0];
        assert_eq!(resource.schema_url, attr::SEMCONV_SCHEMA_URL);
        for metric in &resource.scope_metrics[0].metrics {
            assert!(!metric.name.contains("token.usage"));
            match metric.data.as_ref().unwrap() {
                Data::Histogram(histogram) => {
                    assert_eq!(
                        histogram.aggregation_temporality,
                        AggregationTemporality::Delta as i32
                    );
                    let mut previous_end = None;
                    for point in &histogram.data_points {
                        assert_eq!(point.bucket_counts.len(), point.explicit_bounds.len() + 1);
                        assert_eq!(point.bucket_counts.iter().sum::<u64>(), point.count);
                        let value = point.sum.unwrap();
                        let bucket =
                            point.bucket_counts.iter().position(|count| *count == 1).unwrap();
                        if bucket > 0 {
                            assert!(value > point.explicit_bounds[bucket - 1]);
                        }
                        if bucket < point.explicit_bounds.len() {
                            assert!(value <= point.explicit_bounds[bucket]);
                        }
                        if let Some(end) = previous_end {
                            assert_eq!(point.start_time_unix_nano, end);
                        }
                        previous_end = Some(point.time_unix_nano);
                        assert!(
                            !point
                                .attributes
                                .iter()
                                .any(|attribute| attribute.key == "gen_ai.system")
                        );
                        if preset != Preset::Web {
                            assert!(
                                point
                                    .attributes
                                    .iter()
                                    .any(|attribute| attribute.key == attr::GEN_AI_PROVIDER_NAME)
                            );
                        }
                    }
                }
                Data::Sum(sum) => {
                    assert!(sum.is_monotonic);
                    assert_eq!(sum.aggregation_temporality, AggregationTemporality::Delta as i32);
                }
                _ => panic!("unexpected metric type"),
            }
            if metric.unit == "{token}" {
                let attribute_sets: Vec<_> = match metric.data.as_ref().unwrap() {
                    Data::Histogram(histogram) => {
                        histogram.data_points.iter().map(|point| &point.attributes).collect()
                    }
                    Data::Sum(sum) => {
                        sum.data_points.iter().map(|point| &point.attributes).collect()
                    }
                    _ => panic!("unexpected token metric type"),
                };
                for attributes in attribute_sets {
                    assert!(attributes.iter().any(|attribute| {
                        attribute.key == attr::GEN_AI_TOKEN_MODALITY
                            && matches!(
                                attribute.value.as_ref().and_then(|value| value.value.as_ref()),
                                Some(Value::StringValue(value)) if value == "text"
                            )
                    }));
                }
            }
        }
    }
}

#[test]
fn seeded_scenarios_change_with_seed_and_have_reused_conversations() {
    let config = Config { count: 2, error_rate: 0, ..Config::default() };
    let request = traces::generate(&config, 0, 2);
    let spans = &request.resource_spans[0].scope_spans[0].spans;
    let conversations: Vec<_> = spans
        .iter()
        .filter(|span| span.parent_span_id.is_empty())
        .map(|span| {
            span.attributes
                .iter()
                .find(|attribute| attribute.key == attr::GEN_AI_CONVERSATION_ID)
                .unwrap()
        })
        .collect();
    assert_eq!(conversations[0], conversations[1]);
    let different = Config { seed: config.seed + 1, ..config.clone() };
    assert_ne!(request, traces::generate(&different, 0, 2));
    assert_ne!(logs::generate(&config, 0, 2), logs::generate(&different, 0, 2));
}

#[test]
fn cli_config_rejects_invalid_ranges_and_unknown_fields_before_writes() {
    for flags in [
        vec!["--count", "0"],
        vec!["--batch-size", "0"],
        vec!["--jobs", "0"],
        vec!["--signals", "traces,traces"],
        vec!["--interval-ns", "0"],
        vec!["--error-rate", "101"],
        vec!["--start-ns", "18446744073709551615"],
        vec!["--endpoint", "https://user:secret@example.com", "--output", "s3://bucket/prefix"],
        vec!["--output", "s3://bucket"],
    ] {
        let cli = Cli::try_parse_from(["squasher-signalbox", "generate"].into_iter().chain(flags))
            .unwrap();
        let Command::Generate(generate) = cli.command else { panic!("wrong command") };
        assert!(generate.resolve().is_err());
    }
    assert!(serde_json::from_str::<Config>(r#"{"invented":1}"#).is_err());
}

#[test]
fn schema_advertises_the_same_ranges_as_runtime_validation() {
    let schema = serde_json::to_value(schemars::schema_for!(Config)).unwrap();
    let properties = &schema["properties"];
    assert_eq!(properties["count"]["minimum"], 1);
    assert_eq!(properties["batch_size"]["minimum"], 1);
    assert_eq!(properties["batch_size"]["maximum"], 100_000);
    assert_eq!(properties["jobs"]["maximum"], 256);
    assert_eq!(properties["interval_ns"]["minimum"], 1_000_000_000_u64);
    assert_eq!(properties["error_rate"]["maximum"], 100);
    assert_eq!(properties["signals"]["minItems"], 1);
}

#[test]
fn dry_run_plan_uses_named_signal_counts_for_agents() {
    let config = Config { count: 7, preset: Preset::Rag, ..Config::default() };
    let plan = runner::plan(&config);
    assert_eq!(plan["status"], "planned");
    assert_eq!(plan["counts"]["spans"], 28);
    assert_eq!(plan["counts"]["logs"], 7);
    assert_eq!(plan["counts"]["metric_points"], 35);
    assert!(plan["counts"].is_object());
}
