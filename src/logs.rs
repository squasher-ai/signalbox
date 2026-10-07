use crate::{
    config::Config,
    model::{Record, resource, schema_url, scope},
};
use opentelemetry_proto::tonic::{
    collector::logs::v1::ExportLogsServiceRequest,
    common::v1::{AnyValue, any_value::Value},
    logs::v1::{LogRecord, ResourceLogs, ScopeLogs, SeverityNumber},
};

pub fn generate(config: &Config, start: u64, end: u64) -> ExportLogsServiceRequest {
    let log_records = (start..end)
        .map(|index| {
            let record = Record::new(config, index);
            LogRecord {
                time_unix_nano: record.start + 900_000_000,
                observed_time_unix_nano: record.start + 905_000_000,
                trace_id: record.trace_id.clone(),
                span_id: record.span_ids[0].clone(),
                flags: 1,
                severity_number: if record.failed {
                    SeverityNumber::Error as i32
                } else {
                    SeverityNumber::Info as i32
                },
                severity_text: if record.failed { "ERROR" } else { "INFO" }.into(),
                body: Some(AnyValue {
                    value: Some(Value::StringValue(
                        if record.failed {
                            "Synthetic operation failed"
                        } else {
                            "Synthetic operation completed"
                        }
                        .into(),
                    )),
                }),
                attributes: record.root_attributes(config.preset),
                ..Default::default()
            }
        })
        .collect();
    ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            resource: Some(resource(config)),
            schema_url: crate::semconv::attributes::SEMCONV_SCHEMA_URL.into(),
            scope_logs: vec![ScopeLogs {
                scope: Some(scope()),
                log_records,
                schema_url: schema_url(config.preset).into(),
            }],
        }],
    }
}
