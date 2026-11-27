//! Labels for injected faults, written as `ground_truth.json`.
//!
//! The file is deterministic: it contains no timing or host data, so the same
//! seed and configuration give the same bytes.

use crate::{
    config::{Config, Preset},
    faults::{self, Fault, FaultKind},
    model::{Role, id_salt, service_name},
    traces as names,
};
use serde_json::{Value, json};

const NS_PER_SECOND: u64 = 1_000_000_000;

/// Static facts that an evaluator can expect for one fault kind and preset.
struct Expectation {
    span_names: Vec<&'static str>,
    dependency: Option<&'static str>,
    error_type: Option<&'static str>,
    http_status_code: Option<i64>,
    finish_reason: Option<&'static str>,
    signals: &'static [&'static str],
    indicators: &'static [&'static str],
}

fn expect(kind: FaultKind, preset: Preset) -> Expectation {
    let work = match preset {
        Preset::Agent | Preset::Rag => names::AGENT_CHAT,
        Preset::Web => names::WEB_QUERY,
        Preset::WebTopology => names::API_QUERY,
    };
    let root = match preset {
        Preset::Agent | Preset::Rag => names::AGENT_ROOT,
        Preset::Web => names::WEB_ROOT,
        Preset::WebTopology => names::API_SERVER,
    };
    let dependency = Some(if preset.is_genai() { "openai" } else { "postgresql" });
    let base = Expectation {
        span_names: vec![root],
        dependency: None,
        error_type: None,
        http_status_code: None,
        finish_reason: None,
        signals: &["traces", "logs", "metrics"],
        indicators: &[],
    };
    match kind {
        FaultKind::LatencyRegression => Expectation {
            span_names: vec![work, root],
            signals: &["traces", "metrics"],
            indicators: &["span_duration_increase", "duration_histogram_shift", "version_change"],
            ..base
        },
        FaultKind::ErrorSpike => Expectation {
            error_type: Some("UnhandledException"),
            http_status_code: (!preset.is_genai()).then_some(500),
            indicators: &["error_rate_increase", "error_logs"],
            ..base
        },
        FaultKind::DbSaturation => Expectation {
            span_names: vec![work, root],
            dependency,
            error_type: Some("timeout"),
            indicators: &["dependency_latency_increase", "dependency_timeouts", "error_logs"],
            ..base
        },
        FaultKind::UpstreamOutage => Expectation {
            span_names: if preset.is_genai() {
                vec![work, root]
            } else {
                vec![names::API_SERVER, names::GATEWAY_CLIENT, names::GATEWAY_ROOT]
            },
            dependency: if preset.is_genai() { dependency } else { Some(names_api()) },
            error_type: Some("503"),
            http_status_code: Some(503),
            indicators: &["upstream_5xx", "error_rate_increase", "error_logs"],
            ..base
        },
        FaultKind::LogBurst => Expectation {
            signals: &["logs"],
            indicators: &["log_volume_increase", "warn_logs"],
            ..base
        },
        FaultKind::GenaiRateLimit => Expectation {
            span_names: vec![work, root],
            dependency,
            error_type: Some("rate_limit_exceeded"),
            http_status_code: Some(429),
            indicators: &["provider_rate_limit", "error_rate_increase", "error_logs"],
            ..base
        },
        FaultKind::GenaiContextLength => Expectation {
            span_names: vec![work, root],
            dependency,
            error_type: Some("context_length_exceeded"),
            http_status_code: Some(400),
            indicators: &["input_tokens_exceed_context", "error_rate_increase", "error_logs"],
            ..base
        },
        FaultKind::GenaiRefusal => Expectation {
            span_names: vec![work],
            dependency,
            finish_reason: Some("content_filter"),
            indicators: &["finish_reason_content_filter", "low_output_tokens", "warn_logs"],
            ..base
        },
        FaultKind::GenaiToolError => Expectation {
            span_names: vec![names::AGENT_TOOL],
            error_type: Some("tool_error"),
            signals: &["traces", "logs"],
            indicators: &["tool_span_errors", "error_logs"],
            ..base
        },
    }
}

fn names_api() -> &'static str {
    crate::model::TOPOLOGY_API
}

fn offset_ns(config: &Config, seconds: u64) -> u64 {
    config.start_ns.saturating_add(seconds.saturating_mul(NS_PER_SECOND))
}

fn fault_label(config: &Config, position: usize, fault: &Fault, affected: u64) -> Value {
    let (first, end) = faults::window(config, fault);
    let versions: Vec<&str> = match &fault.version {
        Some(version) => vec![version],
        None => (0..=config.releases.len())
            .filter(|slot| {
                let (from, to) = faults::version_range(config, *slot);
                from.max(first) < to.min(end)
            })
            .map(|slot| faults::version_name(config, slot))
            .collect(),
    };
    let expectation = expect(fault.kind, config.preset);
    json!({
        "id": format!("fault-{}", position + 1),
        "kind": fault.kind,
        "start_s": fault.start_s,
        "end_s": fault.end_s,
        "start_time_unix_nano": offset_ns(config, fault.start_s),
        "end_time_unix_nano": offset_ns(config, fault.end_s),
        "records": { "first": first, "end_exclusive": end },
        "rate": fault.rate(),
        "affected_records": affected,
        "extra_logs": if fault.kind == FaultKind::LogBurst {
            affected * faults::BURST_LOGS_PER_RECORD
        } else {
            0
        },
        "service": service_name(config, Role::Primary),
        "service_versions": versions,
        "span_names": expectation.span_names,
        "dependency": expectation.dependency,
        "error_type": expectation.error_type,
        "http_status_code": expectation.http_status_code,
        "finish_reason": expectation.finish_reason,
        "expected_signals": expectation.signals,
        "indicators": expectation.indicators,
    })
}

pub fn build(config: &Config, affected: &[u64]) -> Value {
    let versions: Vec<Value> = (0..=config.releases.len())
        .map(|slot| {
            let (first, end) = faults::version_range(config, slot);
            json!({
                "version": faults::version_name(config, slot),
                "start_time_unix_nano": config.start_ns + first * config.interval_ns,
                "records": { "first": first, "end_exclusive": end },
            })
        })
        .collect();
    json!({
        "schema_version": 1,
        "run_id": config.run_id,
        "id_salt": format!("{:016x}", id_salt(config)),
        "preset": config.preset,
        "seed": config.seed,
        "records": config.count,
        "start_ns": config.start_ns,
        "interval_ns": config.interval_ns,
        "baseline_error_rate": config.error_rate,
        "service": service_name(config, Role::Primary),
        "versions": versions,
        "faults": config
            .faults
            .iter()
            .zip(affected)
            .enumerate()
            .map(|(position, (fault, affected))| fault_label(config, position, fault, *affected))
            .collect::<Vec<_>>(),
    })
}
