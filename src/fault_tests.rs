use crate::semconv::attributes as attr;
use crate::{
    cli::{Cli, Command},
    config::{Config, Fault, Format, Preset, Progress, Release},
    faults::FaultKind,
    logs, metrics, runner, traces,
};
use clap::Parser;
use opentelemetry_proto::tonic::{
    collector::trace::v1::ExportTraceServiceRequest,
    common::v1::{AnyValue, KeyValue, any_value::Value},
    resource::v1::Resource,
    trace::v1::Span,
};
use prost::Message;
use std::collections::{BTreeMap, BTreeSet};

fn fault(kind: FaultKind, start_s: u64, end_s: u64) -> Fault {
    Fault { kind, start_s, end_s, rate: None, version: None }
}

fn attribute<'a>(attributes: &'a [KeyValue], key: &str) -> Option<&'a Value> {
    attributes
        .iter()
        .find(|attribute| attribute.key == key)
        .and_then(|attribute| attribute.value.as_ref())
        .and_then(|value| value.value.as_ref())
}

fn resource_text(resource: &Option<Resource>, key: &str) -> String {
    match attribute(&resource.as_ref().unwrap().attributes, key) {
        Some(Value::StringValue(value)) => value.clone(),
        other => panic!("missing resource attribute {key}: {other:?}"),
    }
}

fn spans(config: &Config) -> Vec<(String, String, Span)> {
    traces::generate(config, 0, config.count)
        .resource_spans
        .into_iter()
        .flat_map(|group| {
            let service = resource_text(&group.resource, attr::SERVICE_NAME);
            let version = resource_text(&group.resource, attr::SERVICE_VERSION);
            group.scope_spans.into_iter().flat_map(move |scope| {
                let (service, version) = (service.clone(), version.clone());
                scope.spans.into_iter().map(move |span| (service.clone(), version.clone(), span))
            })
        })
        .collect()
}

fn scenario() -> Config {
    Config {
        count: 120,
        preset: Preset::Agent,
        releases: vec![Release { at_s: 60, version: "2.0.0".into() }],
        faults: vec![
            Fault { rate: Some(100), ..fault(FaultKind::GenaiRateLimit, 10, 20) },
            fault(FaultKind::GenaiRefusal, 30, 50),
            fault(FaultKind::GenaiToolError, 40, 60),
            fault(FaultKind::LogBurst, 100, 110),
            Fault { version: Some("2.0.0".into()), ..fault(FaultKind::LatencyRegression, 0, 200) },
        ],
        ..Config::default()
    }
}

#[tokio::test]
async fn fault_runs_are_byte_identical_and_ground_truth_matches_the_signals() {
    let temporary = tempfile::tempdir().unwrap();
    let mut outputs = Vec::new();
    for jobs in [1, 4] {
        let output = temporary.path().join(format!("run-{jobs}"));
        let config = Config {
            batch_size: 7,
            jobs,
            output: output.to_str().unwrap().into(),
            progress: Progress::Never,
            ..scenario()
        };
        let summary = runner::run(config).await.unwrap();
        assert_eq!(summary.logs, 120 + 10 * 20);
        outputs.push((output, summary));
    }
    let names: Vec<_> = std::fs::read_dir(&outputs[0].0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name != "manifest.json")
        .collect();
    assert!(names.iter().any(|name| name == "ground_truth.json"));
    for name in &names {
        assert_eq!(
            std::fs::read(outputs[0].0.join(name)).unwrap(),
            std::fs::read(outputs[1].0.join(name)).unwrap(),
            "{name:?} differs between job counts"
        );
    }
    let truth: serde_json::Value =
        serde_json::from_slice(&std::fs::read(outputs[0].0.join("ground_truth.json")).unwrap())
            .unwrap();
    let faults = truth["faults"].as_array().unwrap();
    let labeled: Vec<_> =
        faults.iter().map(|fault| fault["affected_records"].as_u64().unwrap()).collect();
    assert_eq!(outputs[0].1.affected_records, labeled);
    assert_eq!(faults[0]["affected_records"], 10);
    assert_eq!(faults[3]["extra_logs"], 200);
    assert_eq!(faults[4]["affected_records"], 60);
    assert_eq!(faults[4]["service_versions"], serde_json::json!(["2.0.0"]));

    // Count the labels independently from the decoded traces.
    let config = scenario();
    let mut observed = BTreeMap::<&str, BTreeSet<u64>>::new();
    for (_, version, span) in spans(&config) {
        let index = (span.start_time_unix_nano - config.start_ns) / config.interval_ns;
        let error = attribute(&span.attributes, attr::ERROR_TYPE);
        if span.name == traces::AGENT_CHAT {
            if error == Some(&Value::StringValue("rate_limit_exceeded".into())) {
                observed.entry("rate_limit").or_default().insert(index);
                assert_eq!(
                    attribute(&span.attributes, attr::HTTP_RESPONSE_STATUS_CODE),
                    Some(&Value::IntValue(429))
                );
            }
            if let Some(Value::ArrayValue(reasons)) =
                attribute(&span.attributes, attr::GEN_AI_RESPONSE_FINISH_REASONS)
                && reasons.values[0].value == Some(Value::StringValue("content_filter".into()))
            {
                observed.entry("refusal").or_default().insert(index);
            }
            let slow = span.end_time_unix_nano - span.start_time_unix_nano >= 600_000_000;
            assert_eq!(version == "2.0.0", index >= 60);
            if version == "2.0.0" {
                observed.entry("regression").or_default().insert(index);
                assert!(span.end_time_unix_nano - span.start_time_unix_nano >= 60_000_000);
            } else {
                assert!(!slow, "a 1.0.0 model call is slow");
            }
        }
        if span.name == traces::AGENT_TOOL && error.is_some() {
            observed.entry("tool").or_default().insert(index);
        }
    }
    let windows = [("rate_limit", 10..20), ("refusal", 30..50), ("tool", 40..60)];
    for ((name, window), label) in windows.into_iter().zip(faults) {
        let hits = &observed[name];
        assert!(hits.iter().all(|index| window.contains(index)), "{name} leaked outside window");
        assert_eq!(hits.len() as u64, label["affected_records"].as_u64().unwrap(), "{name}");
    }
    assert_eq!(observed["regression"].len(), 60);
}

#[test]
fn releases_set_service_version_consistently_across_signals() {
    let config = Config {
        count: 30,
        preset: Preset::WebTopology,
        service_version: "3.1.4".into(),
        releases: vec![
            Release { at_s: 10, version: "3.2.0".into() },
            Release { at_s: 20, version: "3.3.0".into() },
        ],
        ..Config::default()
    };
    let versions = |resources: Vec<&Option<Resource>>| -> BTreeSet<(String, String)> {
        resources
            .into_iter()
            .map(|resource| {
                (
                    resource_text(resource, attr::SERVICE_NAME),
                    resource_text(resource, attr::SERVICE_VERSION),
                )
            })
            .collect()
    };
    let trace = traces::generate(&config, 5, 25);
    let log = logs::generate(&config, 5, 25);
    let metric = metrics::generate(&config, 5, 25);
    let trace_versions = versions(trace.resource_spans.iter().map(|r| &r.resource).collect());
    let api: BTreeSet<_> = ["3.1.4", "3.2.0", "3.3.0"]
        .into_iter()
        .map(|version| ("synthetic-api".to_owned(), version.to_owned()))
        .collect();
    assert!(api.is_subset(&trace_versions));
    assert!(trace_versions.contains(&("synthetic-gateway".to_owned(), "3.1.4".to_owned())));
    assert_eq!(api, versions(log.resource_logs.iter().map(|r| &r.resource).collect()));
    assert_eq!(
        trace_versions,
        versions(metric.resource_metrics.iter().map(|r| &r.resource).collect())
    );
}

#[test]
fn topology_spans_form_one_tree_across_two_services() {
    let config = Config { count: 3, preset: Preset::WebTopology, ..Config::default() };
    let spans = spans(&config);
    assert_eq!(spans.len(), 15);
    for (service, _, span) in &spans {
        let parent = spans.iter().find(|(_, _, other)| other.span_id == span.parent_span_id);
        let expected = match span.name.as_str() {
            traces::GATEWAY_ROOT => None,
            traces::GATEWAY_CLIENT => Some(traces::GATEWAY_ROOT),
            traces::API_SERVER => Some(traces::GATEWAY_CLIENT),
            _ => Some(traces::API_SERVER),
        };
        assert_eq!(parent.map(|(_, _, parent)| parent.name.as_str()), expected);
        if let Some((_, _, parent)) = parent {
            assert_eq!(parent.trace_id, span.trace_id);
            assert!(parent.start_time_unix_nano <= span.start_time_unix_nano);
            assert!(parent.end_time_unix_nano >= span.end_time_unix_nano);
        }
        let gateway = matches!(span.name.as_str(), traces::GATEWAY_ROOT | traces::GATEWAY_CLIENT);
        assert_eq!(service, if gateway { "synthetic-gateway" } else { "synthetic-api" });
    }
}

#[test]
fn run_id_changes_identities_but_not_values() {
    let base = Config { count: 50, preset: Preset::Rag, ..Config::default() };
    let ids = |config: &Config| -> BTreeSet<Vec<u8>> {
        spans(config).into_iter().map(|(_, _, span)| span.trace_id).collect()
    };
    let first = Config { run_id: Some("run-a".into()), ..base.clone() };
    let second = Config { run_id: Some("run-b".into()), ..base.clone() };
    assert!(ids(&first).is_disjoint(&ids(&second)));
    assert_eq!(ids(&base), ids(&base.clone()));
    let timing = |config: &Config| -> Vec<_> {
        spans(config)
            .into_iter()
            .map(|(_, _, span)| (span.name, span.start_time_unix_nano, span.end_time_unix_nano))
            .collect()
    };
    assert_eq!(timing(&first), timing(&second));
}

#[test]
fn fault_configuration_errors_are_rejected_before_writes() {
    for flags in [
        vec!["--preset", "web", "--fault", "genai-rate-limit"],
        vec!["--preset", "agent", "--fault", "db-saturation"],
        vec!["--preset", "web", "--fault", "upstream-outage"],
        vec!["--release", "2.0.0@20,1.5.0@10"],
        vec!["--release", "1.0.0@10"],
        vec!["--count", "10", "--release", "2.0.0@10"],
        vec!["--release", "2.0.0"],
        vec!["--service-version", ""],
        vec!["--run-id", "bad\nid"],
        vec!["--send", "http://user:token@localhost:4318"],
        vec!["--send", "http://localhost:4318/?key=secret"],
    ] {
        let parsed = Cli::try_parse_from(["signalbox", "generate"].into_iter().chain(flags));
        let Ok(cli) = parsed else { continue };
        let Command::Generate(generate) = cli.command else { panic!("wrong command") };
        assert!(generate.resolve().is_err());
    }
    for bad in [
        fault(FaultKind::ErrorSpike, 10, 10),
        Fault { rate: Some(0), ..fault(FaultKind::ErrorSpike, 0, 10) },
        Fault { rate: Some(50), ..fault(FaultKind::LogBurst, 0, 10) },
        Fault { version: Some("9.9.9".into()), ..fault(FaultKind::ErrorSpike, 0, 10) },
        fault(FaultKind::ErrorSpike, 20_000, 30_000),
    ] {
        let config = Config { faults: vec![bad], ..Config::default() };
        assert!(config.validate().is_err());
    }
    assert!(
        serde_json::from_str::<Config>(
            r#"{"faults":[{"kind":"error_spike","start_s":0,"end_s":1,"extra":1}]}"#
        )
        .is_err()
    );
}

#[test]
fn fault_presets_scale_with_the_run_and_label_the_release() {
    let cli = Cli::try_parse_from([
        "signalbox",
        "generate",
        "--count",
        "400",
        "--format",
        "json",
        "--fault",
        "release-regression,error-spike",
    ])
    .unwrap();
    let Command::Generate(generate) = cli.command else { panic!("wrong command") };
    let (config, _) = generate.resolve().unwrap();
    assert_eq!(config.format, Format::Json);
    assert_eq!(config.releases, vec![Release { at_s: 200, version: "1.1.0".into() }]);
    assert_eq!(config.faults[0].kind, FaultKind::ErrorSpike);
    assert_eq!((config.faults[0].start_s, config.faults[0].end_s), (40, 180));
    assert_eq!(config.faults[1].version.as_deref(), Some("1.1.0"));
    let plan = runner::plan(&config);
    assert_eq!(plan["fault_windows"][1]["first_record"], 200);
    assert_eq!(plan["fault_windows"][1]["end_record_exclusive"], 400);
    let cli = Cli::try_parse_from([
        "signalbox",
        "generate",
        "--count",
        "100",
        "--preset",
        "web-topology",
        "--fault",
        "db-saturation,upstream-outage",
    ])
    .unwrap();
    let Command::Generate(generate) = cli.command else { panic!("wrong command") };
    let (config, _) = generate.resolve().unwrap();
    let windows: Vec<_> = config.faults.iter().map(|f| (f.start_s, f.end_s)).collect();
    assert_eq!(windows, [(40, 65), (65, 90)]);
    let affected = crate::faults::tally(&config, 0, config.count);
    assert!(affected.iter().all(|hits| *hits > 0), "a preset fault never fired: {affected:?}");
}

#[test]
fn trace_request_round_trips_with_array_attributes() {
    let config = Config { count: 2, error_rate: 0, ..Config::default() };
    let request = traces::generate(&config, 0, 2);
    let decoded = ExportTraceServiceRequest::decode(request.encode_to_vec().as_slice()).unwrap();
    let chat = &decoded.resource_spans[0].scope_spans[0].spans[1];
    let Some(Value::ArrayValue(reasons)) =
        attribute(&chat.attributes, attr::GEN_AI_RESPONSE_FINISH_REASONS)
    else {
        panic!("missing finish reasons")
    };
    assert_eq!(reasons.values, vec![AnyValue { value: Some(Value::StringValue("stop".into())) }]);
}

/// Record indices whose items in `signal` carry `error.type` = `error_type`.
fn erroring_records(config: &Config, signal: &str, error_type: &str) -> BTreeSet<u64> {
    let wanted = Value::StringValue(error_type.into());
    let by_trace = |trace_id: &[u8]| u64::from_be_bytes(trace_id[8..].try_into().unwrap()) - 1;
    let mut found = BTreeSet::new();
    match signal {
        "traces" => {
            for (_, _, span) in spans(config) {
                if attribute(&span.attributes, attr::ERROR_TYPE) == Some(&wanted) {
                    found.insert(by_trace(&span.trace_id));
                }
            }
        }
        "logs" => {
            for group in logs::generate(config, 0, config.count).resource_logs {
                for log in group.scope_logs.into_iter().flat_map(|scope| scope.log_records) {
                    if attribute(&log.attributes, attr::ERROR_TYPE) == Some(&wanted) {
                        found.insert(by_trace(&log.trace_id));
                    }
                }
            }
        }
        "metrics" => {
            use opentelemetry_proto::tonic::metrics::v1::metric::Data;
            let request = metrics::generate(config, 0, config.count);
            for metric in request
                .resource_metrics
                .into_iter()
                .flat_map(|group| group.scope_metrics)
                .flat_map(|scope| scope.metrics)
            {
                let points: Vec<(Vec<KeyValue>, u64)> = match metric.data.unwrap() {
                    Data::Histogram(data) => data
                        .data_points
                        .into_iter()
                        .map(|point| (point.attributes, point.start_time_unix_nano))
                        .collect(),
                    Data::Sum(data) => data
                        .data_points
                        .into_iter()
                        .map(|point| (point.attributes, point.start_time_unix_nano))
                        .collect(),
                    other => panic!("unexpected metric data {other:?}"),
                };
                for (attributes, start) in points {
                    if attribute(&attributes, attr::ERROR_TYPE) == Some(&wanted) {
                        found.insert((start - config.start_ns) / config.interval_ns);
                    }
                }
            }
        }
        other => panic!("unknown signal {other}"),
    }
    found
}

#[test]
fn error_spike_labels_match_every_expected_signal() {
    for preset in [Preset::Agent, Preset::Rag, Preset::Web, Preset::WebTopology] {
        let config = Config {
            count: 80,
            preset,
            error_rate: 0,
            faults: vec![fault(FaultKind::ErrorSpike, 20, 60)],
            ..Config::default()
        };
        let affected = crate::faults::tally(&config, 0, config.count);
        let truth = crate::ground_truth::build(&config, &affected);
        let label = &truth["faults"][0];
        let error_type = label["error_type"].as_str().unwrap();
        let signals = label["expected_signals"].as_array().unwrap();
        assert_eq!(signals.len(), 3, "{preset:?}");
        let mut expected = None;
        for signal in signals {
            let signal = signal.as_str().unwrap();
            let found = erroring_records(&config, signal, error_type);
            assert_eq!(found.len() as u64, affected[0], "{preset:?} {signal}");
            assert!(found.iter().all(|index| (20..60).contains(index)), "{preset:?} {signal}");
            assert_eq!(expected.get_or_insert_with(|| found.clone()), &found, "{preset:?}");
        }
    }
}

#[test]
fn release_regression_uses_the_offset_of_a_configured_release() {
    let cli = Cli::try_parse_from([
        "signalbox",
        "generate",
        "--count",
        "400",
        "--release",
        "1.1.0@120",
        "--fault",
        "release-regression",
    ])
    .unwrap();
    let Command::Generate(generate) = cli.command else { panic!("wrong command") };
    let (config, _) = generate.resolve().unwrap();
    assert_eq!(config.releases, vec![Release { at_s: 120, version: "1.1.0".into() }]);
    let regression = &config.faults[0];
    assert_eq!((regression.start_s, regression.version.as_deref()), (120, Some("1.1.0")));
    let affected = crate::faults::tally(&config, 0, config.count);
    let truth = crate::ground_truth::build(&config, &affected);
    let label = &truth["faults"][0];
    assert_eq!(label["records"]["first"], truth["versions"][1]["records"]["first"]);
    assert_eq!(label["affected_records"], 280);
    // The release serves exactly the labeled scenarios.
    let served: BTreeSet<u64> = spans(&config)
        .into_iter()
        .filter(|(_, _, span)| span.name == traces::AGENT_CHAT)
        .filter(|(_, version, _)| version == "1.1.0")
        .map(|(_, _, span)| (span.start_time_unix_nano - config.start_ns) / config.interval_ns)
        .collect();
    assert_eq!(served, (120..400).collect());
}
