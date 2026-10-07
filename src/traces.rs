use crate::semconv::attributes as attr;
use crate::{
    config::{Config, Preset},
    model::{Record, integer, resource, schema_url, scope, text},
};
use opentelemetry_proto::tonic::{
    collector::trace::v1::ExportTraceServiceRequest,
    trace::v1::{ResourceSpans, ScopeSpans, Span, Status, span::SpanKind, status::StatusCode},
};

pub fn generate(config: &Config, start: u64, end: u64) -> ExportTraceServiceRequest {
    let mut spans =
        Vec::with_capacity((end - start) as usize * config.preset.spans_per_record() as usize);
    for index in start..end {
        let record = Record::new(config, index);
        let mut root = span(&record, 0, None, record.start, 900_000_000);
        root.kind = if config.preset == Preset::Web {
            SpanKind::Server as i32
        } else {
            SpanKind::Internal as i32
        };
        root.name = if config.preset == Preset::Web {
            "GET /synthetic"
        } else {
            "invoke_agent synthetic-assistant"
        }
        .into();
        root.attributes = record.root_attributes(config.preset);
        spans.push(root);
        let mut inference = span(&record, 1, Some(0), record.start + 100_000_000, record.duration);
        inference.kind = SpanKind::Client as i32;
        if config.preset == Preset::Web {
            inference.name = "SELECT synthetic_records".into();
            inference.attributes = vec![
                text(attr::DB_SYSTEM_NAME, "postgresql"),
                text(attr::DB_OPERATION_NAME, "SELECT"),
            ];
            if record.failed {
                inference.attributes.push(text(attr::ERROR_TYPE, "SyntheticError"));
            }
        } else {
            inference.name = "chat synthetic-chat-model".into();
            inference.attributes = record.inference_attributes();
            inference.attributes.extend([
                integer(attr::GEN_AI_USAGE_INPUT_TOKENS, record.input_tokens),
                integer(attr::GEN_AI_USAGE_OUTPUT_TOKENS, record.output_tokens),
            ]);
        }
        spans.push(inference);
        if config.preset != Preset::Web {
            let mut tool = span(&record, 2, Some(0), record.start + 650_000_000, 100_000_000);
            tool.name = "execute_tool lookup_record".into();
            tool.attributes = vec![
                text(attr::GEN_AI_OPERATION_NAME, "execute_tool"),
                text(attr::GEN_AI_TOOL_NAME, "lookup_record"),
                text(attr::GEN_AI_TOOL_TYPE, "function"),
                text(attr::GEN_AI_TOOL_CALL_ID, format!("call-{index}")),
            ];
            spans.push(tool);
        }
        if config.preset == Preset::Rag {
            let mut retrieval = span(&record, 3, Some(0), record.start + 10_000_000, 60_000_000);
            retrieval.name = "retrieval synthetic-documents".into();
            retrieval.attributes = vec![
                text(attr::GEN_AI_OPERATION_NAME, "retrieval"),
                text(attr::GEN_AI_DATA_SOURCE_ID, "synthetic-documents"),
                integer(attr::GEN_AI_RETRIEVAL_TOP_K, 5),
            ];
            spans.push(retrieval);
        }
    }
    ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(resource(config)),
            schema_url: attr::SEMCONV_SCHEMA_URL.into(),
            scope_spans: vec![ScopeSpans {
                scope: Some(scope()),
                spans,
                schema_url: schema_url(config.preset).into(),
            }],
        }],
    }
}

fn span(record: &Record, id: usize, parent: Option<usize>, start: u64, duration: u64) -> Span {
    Span {
        trace_id: record.trace_id.clone(),
        span_id: record.span_ids[id].clone(),
        parent_span_id: parent.map_or_else(Vec::new, |p| record.span_ids[p].clone()),
        start_time_unix_nano: start,
        end_time_unix_nano: start + duration,
        flags: 1,
        kind: SpanKind::Internal as i32,
        status: if record.failed && id <= 1 {
            Some(Status { code: StatusCode::Error as i32, message: "synthetic failure".into() })
        } else {
            None
        },
        ..Default::default()
    }
}
