use crate::error::Error;
use clap::ValueEnum;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Json,
    Protobuf,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Protobuf => "pb",
        }
    }
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Protobuf => "application/x-protobuf",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Preset {
    Agent,
    Rag,
    Web,
}

impl Preset {
    pub fn spans_per_record(self) -> u64 {
        match self {
            Self::Agent => 3,
            Self::Rag => 4,
            Self::Web => 2,
        }
    }
    pub fn points_per_record(self) -> u64 {
        match self {
            Self::Agent | Self::Rag => 5,
            Self::Web => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Signal {
    Traces,
    Logs,
    Metrics,
}

impl Signal {
    pub fn name(self) -> &'static str {
        match self {
            Self::Traces => "traces",
            Self::Logs => "logs",
            Self::Metrics => "metrics",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Progress {
    Auto,
    Always,
    Never,
}

/// A record is one scenario, with correlated spans, one log, and metric points.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    #[schemars(range(min = 1))]
    pub count: u64,
    #[schemars(range(min = 1, max = 100_000))]
    pub batch_size: usize,
    #[schemars(range(min = 1, max = 256))]
    pub jobs: usize,
    pub seed: u64,
    pub preset: Preset,
    #[schemars(length(min = 1))]
    pub signals: Vec<Signal>,
    pub format: Format,
    #[schemars(length(min = 1))]
    pub output: String,
    #[schemars(length(min = 1, max = 256))]
    pub service_name: String,
    #[schemars(range(min = 1))]
    pub start_ns: u64,
    #[schemars(range(min = 1_000_000_000))]
    pub interval_ns: u64,
    #[schemars(range(min = 0, max = 100))]
    pub error_rate: u8,
    pub endpoint: Option<String>,
    #[schemars(length(min = 1))]
    pub region: String,
    pub progress: Progress,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            count: 10_000,
            batch_size: 1_000,
            jobs: std::thread::available_parallelism().map_or(1, usize::from).min(8),
            seed: 42,
            preset: Preset::Agent,
            signals: vec![Signal::Traces, Signal::Logs, Signal::Metrics],
            format: Format::Protobuf,
            output: "otel-data".into(),
            service_name: "synthetic-agent".into(),
            start_ns: 1_759_190_400_000_000_000,
            interval_ns: 1_000_000_000,
            error_rate: 5,
            endpoint: None,
            region: "us-east-1".into(),
            progress: Progress::Auto,
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<(), Error> {
        let invalid = |message| Error::new("invalid_config", message);
        if self.count == 0 || self.batch_size == 0 || self.batch_size > 100_000 {
            return Err(invalid("count must be positive; batch_size must be 1..=100000"));
        }
        if self.jobs == 0
            || self.jobs > 256
            || self.jobs.saturating_mul(self.batch_size) > 1_000_000
        {
            return Err(invalid("jobs must be 1..=256; jobs * batch_size must not exceed 1000000"));
        }
        if self.signals.is_empty()
            || self.signals.iter().enumerate().any(|(i, s)| self.signals[..i].contains(s))
        {
            return Err(invalid("signals must contain one or more unique signal names"));
        }
        if self.start_ns == 0 || self.interval_ns < 1_000_000_000 || self.error_rate > 100 {
            return Err(invalid(
                "start_ns must be positive; interval_ns must be at least 1000000000; error_rate must be 0..=100",
            ));
        }
        if (self.count - 1)
            .checked_mul(self.interval_ns)
            .and_then(|v| self.start_ns.checked_add(v))
            .and_then(|v| v.checked_add(self.interval_ns))
            .is_none()
            || self
                .count
                .checked_mul(self.preset.spans_per_record().max(self.preset.points_per_record()))
                .is_none()
        {
            return Err(invalid("count and timestamps exceed the supported 64-bit range"));
        }
        if self.service_name.is_empty()
            || self.service_name.len() > 256
            || self.service_name.chars().any(char::is_control)
        {
            return Err(invalid(
                "service_name must contain 1..=256 bytes and no control characters",
            ));
        }
        if self.output.is_empty()
            || self.output.chars().any(char::is_control)
            || self.region.is_empty()
        {
            return Err(invalid(
                "output and region must be nonempty and output must not contain control characters",
            ));
        }
        if let Some(endpoint) = &self.endpoint {
            let parsed = url::Url::parse(endpoint)
                .map_err(|_| invalid("endpoint must be an absolute HTTP(S) URL"))?;
            if !matches!(parsed.scheme(), "http" | "https")
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
            {
                return Err(invalid(
                    "endpoint must be an HTTP(S) URL without credentials, query, or fragment",
                ));
            }
            if !self.output.starts_with("s3://") {
                return Err(invalid("endpoint requires an s3:// output"));
            }
        }
        Ok(())
    }

    pub fn shards(&self) -> u64 {
        self.count.div_ceil(self.batch_size as u64)
    }
}
