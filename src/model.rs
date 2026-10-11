use crate::config::{Config, Preset};
use crate::faults::{self, FaultKind};
use crate::semconv::attributes as attr;
use opentelemetry_proto::tonic::{
    common::v1::{AnyValue, ArrayValue, InstrumentationScope, KeyValue, any_value::Value},
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

pub fn texts(key: &str, values: &[&str]) -> KeyValue {
    let values = values
        .iter()
        .map(|value| AnyValue { value: Some(Value::StringValue((*value).into())) })
        .collect();
    KeyValue {
        key: key.into(),
        value: Some(AnyValue { value: Some(Value::ArrayValue(ArrayValue { values })) }),
    }
}

/// The service that emits a span, log, or metric point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The single service of a preset, or the API service of `web-topology`.
    Primary,
    /// The edge service of `web-topology`.
    Gateway,
}

pub const TOPOLOGY_GATEWAY: &str = "synthetic-gateway";
pub const TOPOLOGY_API: &str = "synthetic-api";

/// Service name for a role. Releases and faults apply to the primary service.
pub fn service_name(config: &Config, role: Role) -> &str {
    match (config.preset, role) {
        (Preset::WebTopology, Role::Gateway) => TOPOLOGY_GATEWAY,
        (Preset::WebTopology, Role::Primary) => TOPOLOGY_API,
        _ => &config.service_name,
    }
}

/// The resource for one service and version slot (see [`faults::version_slot`]).
pub fn resource(config: &Config, role: Role, version: usize) -> Resource {
    let version = match role {
        Role::Primary => faults::version_name(config, version),
        Role::Gateway => &config.service_version,
    };
    Resource {
        attributes: vec![
            text(attr::SERVICE_NAME, service_name(config, role)),
            text(attr::SERVICE_VERSION, version),
            text(attr::DEPLOYMENT_ENVIRONMENT_NAME, "synthetic"),
            boolean(attr::SQUASHER_SYNTHETIC, true),
            text(attr::SQUASHER_GENERATOR_NAME, env!("CARGO_PKG_NAME")),
            text(attr::SQUASHER_GENERATOR_VERSION, env!("CARGO_PKG_VERSION")),
        ],
        ..Default::default()
    }
}

/// Items grouped by resource in first-seen order. A shard has few groups: at
/// most two services times the versions active in the shard.
pub struct Groups<T>(pub Vec<((Role, usize), T)>);

impl<T: Default> Groups<T> {
    pub fn new() -> Self {
        Self(Vec::new())
    }
    pub fn get(&mut self, role: Role, version: usize) -> &mut T {
        let key = (role, if role == Role::Gateway { 0 } else { version });
        let position = match self.0.iter().position(|(existing, _)| *existing == key) {
            Some(position) => position,
            None => {
                self.0.push((key, T::default()));
                self.0.len() - 1
            }
        };
        &mut self.0[position].1
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
    if preset.is_genai() {
        crate::semconv::attributes::GENAI_SEMCONV_SCHEMA_URL
    } else {
        crate::semconv::attributes::SEMCONV_SCHEMA_URL
    }
}

pub(crate) fn random(seed: u64, index: u64, lane: u64) -> u64 {
    mix(seed
        .wrapping_add(index.wrapping_mul(0x9e3779b97f4a7c15))
        .wrapping_add(lane.wrapping_mul(0xd1b54a32d192ed03)))
}

/// SplitMix64 finalizer. It is a bijection on `u64`, so XOR with distinct
/// salts before mixing gives distinct outputs for the same input.
fn mix(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

/// The identity salt. An explicit `run_id` is hashed with FNV-1a; otherwise
/// the salt is derived from the seed.
pub fn id_salt(config: &Config) -> u64 {
    match &config.run_id {
        Some(run_id) => run_id.bytes().fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        }),
        None => random(config.seed, 0, 0x5eed),
    }
}

pub const ROOT_DURATION_NS: u64 = 900_000_000;

pub struct Record {
    pub trace_id: Vec<u8>,
    pub span_ids: [Vec<u8>; 5],
    pub start: u64,
    /// Duration of the work span (model call or database query).
    pub duration: u64,
    /// Fault delay added to the work span; later spans move by the same amount.
    pub delay: u64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub failed: bool,
    /// The work span itself reports the error (provider or database failure).
    pub work_failed: bool,
    pub error_type: &'static str,
    pub status_code: Option<i64>,
    pub finish_reason: Option<&'static str>,
    pub tool_failed: bool,
    pub burst_logs: u64,
    pub version: usize,
    pub conversation: u64,
}

impl Record {
    pub fn new(config: &Config, index: u64) -> Self {
        let seed = config.seed;
        let salt = id_salt(config);
        let mut trace_id = mix(salt ^ random(seed, index, 0)).to_be_bytes().to_vec();
        trace_id.extend_from_slice(&index.wrapping_add(1).to_be_bytes());
        let failed = random(seed, index, 10) % 100 < config.error_rate as u64;
        let mut record = Self {
            trace_id,
            span_ids: std::array::from_fn(|i| {
                (mix(salt ^ random(seed, index, 100 + i as u64)) | 1).to_be_bytes().to_vec()
            }),
            start: config.start_ns + index * config.interval_ns,
            duration: 10_000_000 + random(seed, index, 11) % 490_000_000,
            delay: 0,
            input_tokens: 256 + (random(seed, index, 12) % 3840) as i64,
            output_tokens: if failed { 0 } else { 32 + (random(seed, index, 13) % 992) as i64 },
            failed,
            work_failed: failed,
            error_type: "SyntheticError",
            status_code: None,
            finish_reason: None,
            tool_failed: false,
            burst_logs: faults::burst_logs(config, index),
            version: faults::version_slot(config, index),
            conversation: mix(salt ^ random(seed, index / 10, 99)),
        };
        if let Some(fault) = faults::select(config, index, record.version) {
            record.apply(config.faults[fault].kind, config.preset, random(seed, index, 14));
        }
        if config.preset.is_genai() && !record.failed && record.finish_reason.is_none() {
            record.finish_reason = Some("stop");
        }
        record
    }

    fn fail(&mut self, error_type: &'static str, status_code: Option<i64>, work_failed: bool) {
        self.failed = true;
        self.work_failed = work_failed;
        self.error_type = error_type;
        self.status_code = status_code;
        self.output_tokens = 0;
    }

    fn apply(&mut self, kind: FaultKind, preset: Preset, draw: u64) {
        match kind {
            FaultKind::LatencyRegression => self.delay = self.duration * 5,
            FaultKind::ErrorSpike => self.fail("UnhandledException", None, false),
            FaultKind::DbSaturation => {
                self.delay = 1_500_000_000 + draw % 1_500_000_000;
                if draw.is_multiple_of(4) {
                    self.fail("timeout", None, true);
                }
            }
            FaultKind::UpstreamOutage => {
                // The model provider (agent presets) or the API behind the gateway.
                self.fail("503", Some(503), preset.is_genai());
                self.input_tokens = 0;
            }
            FaultKind::GenaiRateLimit => {
                self.fail("rate_limit_exceeded", Some(429), true);
                self.input_tokens = 0;
            }
            FaultKind::GenaiContextLength => {
                self.fail("context_length_exceeded", Some(400), true);
                self.input_tokens = 128_001 + (draw % 64_000) as i64;
            }
            FaultKind::GenaiRefusal => {
                self.failed = false;
                self.work_failed = false;
                self.finish_reason = Some("content_filter");
                self.output_tokens = 1 + (draw % 24) as i64;
            }
            FaultKind::GenaiToolError => {
                self.failed = false;
                self.work_failed = false;
                self.tool_failed = true;
            }
            FaultKind::LogBurst => {}
        }
    }

    pub fn work_duration(&self) -> u64 {
        self.duration + self.delay
    }

    pub fn root_duration(&self) -> u64 {
        ROOT_DURATION_NS + self.delay
    }

    /// HTTP status of the request handled by the primary service.
    pub fn http_status(&self) -> i64 {
        if self.failed { self.status_code.unwrap_or(500) } else { 200 }
    }

    pub fn inference_attributes(&self) -> Vec<KeyValue> {
        let mut attributes = vec![
            text(attr::GEN_AI_OPERATION_NAME, "chat"),
            text(attr::GEN_AI_PROVIDER_NAME, "openai"),
            text(attr::GEN_AI_REQUEST_MODEL, "synthetic-chat-model"),
            text(attr::GEN_AI_RESPONSE_MODEL, "synthetic-chat-model"),
        ];
        if self.work_failed {
            attributes.push(text(attr::ERROR_TYPE, self.error_type));
        }
        attributes
    }

    pub fn root_attributes(&self, preset: Preset) -> Vec<KeyValue> {
        let mut attributes = match preset {
            Preset::Web => vec![
                text(attr::HTTP_REQUEST_METHOD, "GET"),
                text(attr::HTTP_ROUTE, "/synthetic"),
                integer(attr::HTTP_RESPONSE_STATUS_CODE, self.http_status()),
            ],
            Preset::WebTopology => vec![
                text(attr::HTTP_REQUEST_METHOD, "GET"),
                text(attr::HTTP_ROUTE, crate::traces::API_ROUTE),
                integer(attr::HTTP_RESPONSE_STATUS_CODE, self.http_status()),
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
            attributes.push(text(attr::ERROR_TYPE, self.error_type));
        }
        attributes
    }
}
