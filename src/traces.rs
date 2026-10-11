use crate::semconv::attributes as attr;
use crate::{
    config::{Config, Preset},
    model::{Groups, Record, Role, integer, resource, schema_url, scope, text, texts},
};
use opentelemetry_proto::tonic::{
    collector::trace::v1::ExportTraceServiceRequest,
    trace::v1::{ResourceSpans, ScopeSpans, Span, Status, span::SpanKind, status::StatusCode},
};

pub const AGENT_ROOT: &str = "invoke_agent synthetic-assistant";
pub const AGENT_CHAT: &str = "chat synthetic-chat-model";
pub const AGENT_TOOL: &str = "execute_tool lookup_record";
pub const AGENT_RETRIEVAL: &str = "retrieval synthetic-documents";
pub const WEB_ROOT: &str = "GET /synthetic";
pub const WEB_QUERY: &str = "SELECT synthetic_records";
pub const GATEWAY_ROUTE: &str = "/checkout";
pub const GATEWAY_ROOT: &str = "GET /checkout";
pub const GATEWAY_CLIENT: &str = "GET";
pub const API_ROUTE: &str = "/orders";
pub const API_SERVER: &str = "GET /orders";
pub const API_QUERY: &str = "SELECT orders";
pub const API_CACHE: &str = "GET sessions";

const MS: u64 = 1_000_000;

pub fn generate(config: &Config, start: u64, end: u64) -> ExportTraceServiceRequest {
    let mut groups: Groups<Vec<Span>> = Groups::new();
    for index in start..end {
        let record = Record::new(config, index);
        match config.preset {
            Preset::WebTopology => topology(&record, &mut groups),
            Preset::Web => web(&record, groups.get(Role::Primary, record.version)),
            Preset::Agent | Preset::Rag => {
                agent(config.preset, index, &record, groups.get(Role::Primary, record.version))
            }
        }
    }
    ExportTraceServiceRequest {
        resource_spans: groups
            .0
            .into_iter()
            .map(|((role, version), spans)| ResourceSpans {
                resource: Some(resource(config, role, version)),
                schema_url: attr::SEMCONV_SCHEMA_URL.into(),
                scope_spans: vec![ScopeSpans {
                    scope: Some(scope()),
                    spans,
                    schema_url: schema_url(config.preset).into(),
                }],
            })
            .collect(),
    }
}

fn agent(preset: Preset, index: u64, record: &Record, spans: &mut Vec<Span>) {
    let mut root = span(record, 0, None, 0, record.root_duration(), record.failed);
    root.name = AGENT_ROOT.into();
    root.attributes = record.root_attributes(preset);
    spans.push(root);
    let mut chat = span(record, 1, Some(0), 100 * MS, record.work_duration(), record.work_failed);
    chat.kind = SpanKind::Client as i32;
    chat.name = AGENT_CHAT.into();
    chat.attributes = record.inference_attributes();
    chat.attributes.extend([
        integer(attr::GEN_AI_USAGE_INPUT_TOKENS, record.input_tokens),
        integer(attr::GEN_AI_USAGE_OUTPUT_TOKENS, record.output_tokens),
    ]);
    if let Some(reason) = record.finish_reason {
        chat.attributes.push(texts(attr::GEN_AI_RESPONSE_FINISH_REASONS, &[reason]));
    }
    if let Some(status) = record.status_code {
        chat.attributes.push(integer(attr::HTTP_RESPONSE_STATUS_CODE, status));
    }
    spans.push(chat);
    let mut tool = span(record, 2, Some(0), 650 * MS + record.delay, 100 * MS, record.tool_failed);
    tool.name = AGENT_TOOL.into();
    tool.attributes = vec![
        text(attr::GEN_AI_OPERATION_NAME, "execute_tool"),
        text(attr::GEN_AI_TOOL_NAME, "lookup_record"),
        text(attr::GEN_AI_TOOL_TYPE, "function"),
        text(attr::GEN_AI_TOOL_CALL_ID, format!("call-{index}")),
    ];
    if record.tool_failed {
        tool.attributes.push(text(attr::ERROR_TYPE, "tool_error"));
    }
    spans.push(tool);
    if preset == Preset::Rag {
        let mut retrieval = span(record, 3, Some(0), 10 * MS, 60 * MS, false);
        retrieval.name = AGENT_RETRIEVAL.into();
        retrieval.attributes = vec![
            text(attr::GEN_AI_OPERATION_NAME, "retrieval"),
            text(attr::GEN_AI_DATA_SOURCE_ID, "synthetic-documents"),
            integer(attr::GEN_AI_RETRIEVAL_TOP_K, 5),
        ];
        spans.push(retrieval);
    }
}

fn web(record: &Record, spans: &mut Vec<Span>) {
    let mut root = span(record, 0, None, 0, record.root_duration(), record.failed);
    root.kind = SpanKind::Server as i32;
    root.name = WEB_ROOT.into();
    root.attributes = record.root_attributes(Preset::Web);
    spans.push(root);
    spans.push(query(record, 1, 0, WEB_QUERY, "synthetic_records"));
}

fn topology(record: &Record, groups: &mut Groups<Vec<Span>>) {
    let status = record.http_status();
    let http = |route: Option<&str>| {
        let mut attributes = vec![text(attr::HTTP_REQUEST_METHOD, "GET")];
        if let Some(route) = route {
            attributes.push(text(attr::HTTP_ROUTE, route));
        } else {
            attributes.push(text(attr::SERVER_ADDRESS, crate::model::TOPOLOGY_API));
        }
        attributes.push(integer(attr::HTTP_RESPONSE_STATUS_CODE, status));
        if record.failed {
            attributes.push(text(attr::ERROR_TYPE, record.error_type));
        }
        attributes
    };
    let root_duration = record.root_duration();
    let gateway = groups.get(Role::Gateway, record.version);
    let mut root = span(record, 0, None, 0, root_duration, record.failed);
    root.kind = SpanKind::Server as i32;
    root.name = GATEWAY_ROOT.into();
    root.attributes = http(Some(GATEWAY_ROUTE));
    gateway.push(root);
    let mut client = span(record, 1, Some(0), 20 * MS, root_duration - 40 * MS, record.failed);
    client.kind = SpanKind::Client as i32;
    client.name = GATEWAY_CLIENT.into();
    client.attributes = http(None);
    gateway.push(client);
    let api = groups.get(Role::Primary, record.version);
    let mut server = span(record, 2, Some(1), 30 * MS, root_duration - 60 * MS, record.failed);
    server.kind = SpanKind::Server as i32;
    server.name = API_SERVER.into();
    server.attributes = http(Some(API_ROUTE));
    api.push(server);
    let mut cache = span(record, 4, Some(2), 40 * MS, 5 * MS, false);
    cache.kind = SpanKind::Client as i32;
    cache.name = API_CACHE.into();
    cache.attributes =
        vec![text(attr::DB_SYSTEM_NAME, "redis"), text(attr::DB_OPERATION_NAME, "GET")];
    api.push(cache);
    api.push(query(record, 3, 2, API_QUERY, "orders"));
}

fn query(record: &Record, id: usize, parent: usize, name: &str, table: &str) -> Span {
    let failed = record.work_failed;
    let mut query = span(record, id, Some(parent), 100 * MS, record.work_duration(), failed);
    query.kind = SpanKind::Client as i32;
    query.name = name.into();
    query.attributes = vec![
        text(attr::DB_SYSTEM_NAME, "postgresql"),
        text(attr::DB_OPERATION_NAME, "SELECT"),
        text(attr::DB_COLLECTION_NAME, table),
    ];
    if failed {
        query.attributes.push(text(attr::ERROR_TYPE, record.error_type));
    }
    query
}

fn span(
    record: &Record,
    id: usize,
    parent: Option<usize>,
    offset: u64,
    duration: u64,
    failed: bool,
) -> Span {
    let start = record.start + offset;
    Span {
        trace_id: record.trace_id.clone(),
        span_id: record.span_ids[id].clone(),
        parent_span_id: parent.map_or_else(Vec::new, |p| record.span_ids[p].clone()),
        start_time_unix_nano: start,
        end_time_unix_nano: start + duration,
        flags: 1,
        kind: SpanKind::Internal as i32,
        status: if failed {
            Some(Status { code: StatusCode::Error as i32, message: "synthetic failure".into() })
        } else {
            None
        },
        ..Default::default()
    }
}
