use crate::{
    config::{Config, Format, Preset, Progress, Signal},
    error::Error,
};
use clap::{Args, Parser, Subcommand};
use std::{
    io::{self, Read},
    path::PathBuf,
};

#[derive(Parser)]
#[command(
    name = "otel-agent-forge",
    version,
    about = "Fast deterministic OTLP traces, logs, and metrics",
    long_about = "Generate correlated synthetic OTLP export requests in bounded shards. Progress goes to stderr; stdout contains one JSON result. Existing output is never overwritten. Use schema to inspect JSON configuration."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Print the JSON configuration schema and default values.
    Schema,
    /// List available scenarios and the signal counts for each record.
    Presets,
    /// Generate data into a new local directory or S3-compatible prefix.
    Generate(Box<Generate>),
}

#[derive(Args)]
pub struct Generate {
    /// JSON configuration file, or - to read stdin. Flags override its fields.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Validate and print the plan without creating output or contacting S3.
    #[arg(long)]
    pub dry_run: bool,
    #[command(flatten)]
    pub options: Overrides,
}

#[derive(Args, Default)]
pub struct Overrides {
    /// Number of scenarios (not spans); default 10000.
    #[arg(long)]
    pub count: Option<u64>,
    /// Scenarios per shard; default 1000; range 1..=100000.
    #[arg(long)]
    pub batch_size: Option<usize>,
    /// Parallel generation/write workers; default min(CPU count, 8).
    #[arg(long)]
    pub jobs: Option<usize>,
    /// Deterministic seed; default 42.
    #[arg(long)]
    pub seed: Option<u64>,
    /// Scenario: agent (3 spans), rag (4 spans), web (2 spans).
    #[arg(long, value_enum)]
    pub preset: Option<Preset>,
    /// Comma-separated signals; default traces,logs,metrics.
    #[arg(long, value_enum, value_delimiter = ',')]
    pub signals: Option<Vec<Signal>>,
    /// OTLP wire format; default protobuf.
    #[arg(long, value_enum)]
    pub format: Option<Format>,
    /// New directory or s3://bucket/new-prefix; default otel-data.
    #[arg(long)]
    pub output: Option<String>,
    /// Resource service name; default synthetic-agent.
    #[arg(long)]
    pub service_name: Option<String>,
    /// First scenario Unix time in nanoseconds; fixed default for reproducibility.
    #[arg(long)]
    pub start_ns: Option<u64>,
    /// Nanoseconds between scenarios; default 1000000000; minimum 1000000000.
    #[arg(long)]
    pub interval_ns: Option<u64>,
    /// Synthetic failure percentage; default 5; range 0..=100.
    #[arg(long)]
    pub error_rate: Option<u8>,
    /// S3-compatible HTTP(S) endpoint. Requires AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY; profile credentials are not forwarded.
    #[arg(long)]
    pub endpoint: Option<String>,
    /// S3 signing region; default us-east-1.
    #[arg(long)]
    pub region: Option<String>,
    /// stderr progress bar: auto (TTY only), always, never.
    #[arg(long, value_enum)]
    pub progress: Option<Progress>,
}

impl Generate {
    pub fn resolve(self) -> Result<(Config, bool), Error> {
        let mut config = match self.config {
            Some(path) => {
                let mut reader: Box<dyn Read> = if path.as_os_str() == "-" {
                    Box::new(io::stdin())
                } else {
                    Box::new(std::fs::File::open(path).map_err(|_| {
                        Error::new("config_read_failed", "Could not open config file")
                    })?)
                };
                let mut bytes = Vec::new();
                reader
                    .by_ref()
                    .take(1_048_577)
                    .read_to_end(&mut bytes)
                    .map_err(|_| Error::new("config_read_failed", "Could not read config"))?;
                if bytes.len() > 1_048_576 {
                    return Err(Error::new("invalid_config", "Config exceeds 1 MiB"));
                }
                serde_json::from_slice(&bytes)
                    .map_err(|error| Error::new("invalid_config", error.to_string()))?
            }
            None => Config::default(),
        };
        macro_rules! override_fields {
            ($($field:ident),+) => { $(if let Some(value) = self.options.$field { config.$field = value; })+ };
        }
        override_fields!(
            count,
            batch_size,
            jobs,
            seed,
            preset,
            signals,
            format,
            output,
            service_name,
            start_ns,
            interval_ns,
            error_rate,
            region,
            progress
        );
        if let Some(endpoint) = self.options.endpoint {
            config.endpoint = Some(endpoint);
        }
        config.validate()?;
        crate::sink::Sink::validate_output(&config.output)?;
        Ok((config, self.dry_run))
    }
}
