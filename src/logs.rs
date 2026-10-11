use crate::{
    config::{Config, Preset},
    model::{Groups, Record, Role, resource, schema_url, scope, text},
    semconv::attributes as attr,
};
use opentelemetry_proto::tonic::{
    collector::logs::v1::ExportLogsServiceRequest,
    common::v1::{AnyValue, KeyValue, any_value::Value},
    logs::v1::{LogRecord, ResourceLogs, ScopeLogs, SeverityNumber},
};

pub fn generate(config: &Config, start: u64, end: u64) -> ExportLogsServiceRequest {
    let mut groups: Groups<Vec<LogRecord>> = Groups::new();
    for index in start..end {
        let record = Record::new(config, index);
        // The primary service logs from its server or agent span.
        let span = if config.preset == Preset::WebTopology { 2 } else { 0 };
        let logs = groups.get(Role::Primary, record.version);
        let (severity, body) = outcome(&record);
        let mut attributes = record.root_attributes(config.preset);
        if record.tool_failed {
            attributes.push(text(attr::ERROR_TYPE, "tool_error"));
        }
        let time = record.start + record.root_duration();
        logs.push(log(&record, span, time, severity, body, attributes));
        let step = record.root_duration() / (record.burst_logs + 1);
        for attempt in 1..=record.burst_logs {
            logs.push(log(
                &record,
                span,
                record.start + step * attempt,
                SeverityNumber::Warn,
                "Retrying dependency call after a transient error",
                Vec::new(),
            ));
        }
    }
    ExportLogsServiceRequest {
        resource_logs: groups
            .0
            .into_iter()
            .map(|((role, version), log_records)| ResourceLogs {
                resource: Some(resource(config, role, version)),
                schema_url: attr::SEMCONV_SCHEMA_URL.into(),
                scope_logs: vec![ScopeLogs {
                    scope: Some(scope()),
                    log_records,
                    schema_url: schema_url(config.preset).into(),
                }],
            })
            .collect(),
    }
}

fn outcome(record: &Record) -> (SeverityNumber, &'static str) {
    if record.failed {
        let body = match record.error_type {
            "UnhandledException" => "Unhandled exception while processing the request",
            "timeout" => "Database query timed out",
            "503" => "Upstream service unavailable",
            "rate_limit_exceeded" => "Model provider rate limit exceeded",
            "context_length_exceeded" => "Model request exceeded the context window",
            _ => "Synthetic operation failed",
        };
        (SeverityNumber::Error, body)
    } else if record.tool_failed {
        (SeverityNumber::Error, "Tool call failed: lookup_record")
    } else if record.finish_reason == Some("content_filter") {
        (SeverityNumber::Warn, "Model response was filtered")
    } else {
        (SeverityNumber::Info, "Synthetic operation completed")
    }
}

fn log(
    record: &Record,
    span: usize,
    time: u64,
    severity: SeverityNumber,
    body: &str,
    attributes: Vec<KeyValue>,
) -> LogRecord {
    LogRecord {
        time_unix_nano: time,
        observed_time_unix_nano: time + 5_000_000,
        trace_id: record.trace_id.clone(),
        span_id: record.span_ids[span].clone(),
        flags: 1,
        severity_number: severity as i32,
        severity_text: match severity {
            SeverityNumber::Error => "ERROR",
            SeverityNumber::Warn => "WARN",
            _ => "INFO",
        }
        .into(),
        body: Some(AnyValue { value: Some(Value::StringValue(body.into())) }),
        attributes,
        ..Default::default()
    }
}
