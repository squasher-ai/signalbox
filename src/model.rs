use crate::config::{Config, Preset};
use crate::semconv::attributes as attr;
use opentelemetry_proto::tonic::{
    common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value::Value},
    resource::v1::Resource,
};

pub fn text(key: &str, value: impl Into<String>) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(AnyValue { value: Some(Value::StringValue(value.into())) }),
    }
}

pub fn integer(key: &str, value: i64) -> KeyValue {
    KeyValue { key: key.into(), value: Some(AnyValue { value: Some(Value::IntValue(value)) }) }
}

pub fn boolean(key: &str, value: bool) -> KeyValue {
    KeyValue { key: key.into(), value: Some(AnyValue { value: Some(Value::BoolValue(value)) }) }
}

pub fn resource(config: &Config) -> Resource {
    Resource {
        attributes: vec![
            text(attr::SERVICE_NAME, &config.service_name),
            text(attr::SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
            text(attr::DEPLOYMENT_ENVIRONMENT_NAME, "synthetic"),
            boolean(attr::SQUASHER_SYNTHETIC, true),
            text(attr::SQUASHER_GENERATOR_NAME, env!("CARGO_PKG_NAME")),
            text(attr::SQUASHER_GENERATOR_VERSION, env!("CARGO_PKG_VERSION")),
        ],
        ..Default::default()
    }
}

pub fn scope() -> InstrumentationScope {
    InstrumentationScope {
        name: "signalbox".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        ..Default::default()
    }
}

pub fn schema_url(preset: Preset) -> &'static str {
    if preset == Preset::Web {
        crate::semconv::attributes::SEMCONV_SCHEMA_URL
    } else {
        crate::semconv::attributes::GENAI_SEMCONV_SCHEMA_URL
    }
}

fn random(seed: u64, index: u64, lane: u64) -> u64 {
    let mut value = seed
        .wrapping_add(index.wrapping_mul(0x9e3779b97f4a7c15))
        .wrapping_add(lane.wrapping_mul(0xd1b54a32d192ed03));
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

pub struct Record {
    pub trace_id: Vec<u8>,
    pub span_ids: [Vec<u8>; 4],
    pub start: u64,
    pub duration: u64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub failed: bool,
    pub conversation: u64,
}

impl Record {
    pub fn new(config: &Config, index: u64) -> Self {
        let mut trace_id = random(config.seed, index, 0).to_be_bytes().to_vec();
        trace_id.extend_from_slice(&index.wrapping_add(1).to_be_bytes());
        let failed = random(config.seed, index, 10) % 100 < config.error_rate as u64;
        Self {
            trace_id,
            span_ids: std::array::from_fn(|i| {
                (random(config.seed, index, 100 + i as u64) | 1).to_be_bytes().to_vec()
            }),
            start: config.start_ns + index * config.interval_ns,
            duration: 10_000_000 + random(config.seed, index, 11) % 490_000_000,
            input_tokens: 256 + (random(config.seed, index, 12) % 3840) as i64,
            output_tokens: if failed {
                0
            } else {
                32 + (random(config.seed, index, 13) % 992) as i64
            },
            failed,
            conversation: random(config.seed, index / 10, 99),
        }
    }

    pub fn inference_attributes(&self) -> Vec<KeyValue> {
        let mut attributes = vec![
            text(attr::GEN_AI_OPERATION_NAME, "chat"),
            text(attr::GEN_AI_PROVIDER_NAME, "openai"),
            text(attr::GEN_AI_REQUEST_MODEL, "synthetic-chat-model"),
            text(attr::GEN_AI_RESPONSE_MODEL, "synthetic-chat-model"),
        ];
        if self.failed {
            attributes.push(text(attr::ERROR_TYPE, "SyntheticError"));
        }
        attributes
    }

    pub fn root_attributes(&self, preset: Preset) -> Vec<KeyValue> {
        let mut attributes = match preset {
            Preset::Web => vec![
                text(attr::HTTP_REQUEST_METHOD, "GET"),
                text(attr::HTTP_ROUTE, "/synthetic"),
                integer(attr::HTTP_RESPONSE_STATUS_CODE, if self.failed { 500 } else { 200 }),
            ],
            Preset::Agent | Preset::Rag => vec![
                text(attr::GEN_AI_OPERATION_NAME, "invoke_agent"),
                text(attr::GEN_AI_AGENT_NAME, "synthetic-assistant"),
                text(attr::GEN_AI_AGENT_ID, "synthetic-agent-1"),
                text(
                    attr::GEN_AI_CONVERSATION_ID,
                    format!("synthetic-conversation-{}", self.conversation),
                ),
            ],
        };
        if self.failed {
            attributes.push(text(attr::ERROR_TYPE, "SyntheticError"));
        }
        attributes
    }
}
