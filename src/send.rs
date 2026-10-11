//! Direct OTLP/HTTP export.
//!
//! Shards are posted to `{base}/v1/{traces,logs,metrics}` with gzip encoding.
//! Header values come only from `SIGNALBOX_OTLP_HEADERS` and are never written
//! to output, errors, or manifests.

use crate::{
    config::{Format, Signal},
    error::Error,
};
use flate2::{Compression, write::GzEncoder};
use opentelemetry_proto::tonic::collector::{
    logs::v1::ExportLogsServiceResponse, metrics::v1::ExportMetricsServiceResponse,
    trace::v1::ExportTraceServiceResponse,
};
use prost::Message;
use reqwest::header::{CONTENT_ENCODING, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

pub const HEADERS_ENV: &str = "SIGNALBOX_OTLP_HEADERS";
const MAX_ATTEMPTS: u32 = 6;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);
/// Largest success response body read for partial-success counts. OTLP
/// responses are tiny; a larger body is not read into memory.
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SendSummary {
    pub endpoint: String,
    pub requests: u64,
    pub retries: u64,
    pub bytes_sent: u64,
    pub rejected_spans: u64,
    pub rejected_logs: u64,
    pub rejected_data_points: u64,
}

pub fn validate_base(base: &str) -> Result<(), Error> {
    let parsed = url::Url::parse(base);
    match parsed {
        Ok(url)
            if matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none() =>
        {
            Ok(())
        }
        _ => Err(Error::new(
            "invalid_config",
            "send must be an HTTP(S) base URL without credentials, query, or fragment; put credentials in SIGNALBOX_OTLP_HEADERS",
        )),
    }
}

pub fn endpoint(base: &str, signal: Signal) -> String {
    format!("{}/v1/{}", base.trim_end_matches('/'), signal.name())
}

/// Parse `k=v,k2=v2` with percent-encoded values, as in the OTLP exporter
/// header variables. Errors name only the entry position.
pub fn parse_headers(raw: &str) -> Result<HeaderMap, Error> {
    let mut headers = HeaderMap::new();
    for (position, entry) in raw.split(',').enumerate().filter(|(_, e)| !e.trim().is_empty()) {
        let invalid = || {
            Error::new(
                "invalid_headers",
                format!("{HEADERS_ENV} entry {} must be name=value", position + 1),
            )
        };
        let (name, value) = entry.split_once('=').ok_or_else(invalid)?;
        let name = HeaderName::from_bytes(name.trim().as_bytes()).map_err(|_| invalid())?;
        if [CONTENT_TYPE, CONTENT_ENCODING, reqwest::header::CONTENT_LENGTH].contains(&name) {
            return Err(Error::new(
                "invalid_headers",
                format!("{HEADERS_ENV} entry {} sets a header Signalbox controls", position + 1),
            ));
        }
        let decoded = percent_decode(value.trim()).ok_or_else(invalid)?;
        let mut value = HeaderValue::from_bytes(&decoded).map_err(|_| invalid())?;
        value.set_sensitive(true);
        headers.append(name, value);
    }
    Ok(headers)
}

fn percent_decode(value: &str) -> Option<Vec<u8>> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut position = 0;
    while position < bytes.len() {
        if bytes[position] == b'%' {
            let hex = std::str::from_utf8(bytes.get(position + 1..position + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            position += 3;
        } else {
            decoded.push(bytes[position]);
            position += 1;
        }
    }
    Some(decoded)
}

pub fn gzip(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let mut encoder = GzEncoder::new(Vec::with_capacity(bytes.len() / 4), Compression::fast());
    encoder
        .write_all(bytes)
        .and_then(|()| encoder.finish())
        .map_err(|_| Error::new("encode_failed", "Cannot compress an OTLP request"))
}

pub struct Sender {
    client: reqwest::Client,
    base: String,
    requests: AtomicU64,
    retries: AtomicU64,
    bytes_sent: AtomicU64,
    rejected: [AtomicU64; 3],
}

impl Sender {
    pub fn new(base: &str, headers: HeaderMap) -> Result<Self, Error> {
        validate_base(base)?;
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            // Ignore HTTP_PROXY, HTTPS_PROXY, and ALL_PROXY so payloads and
            // credential headers go only to the configured endpoint.
            .no_proxy()
            .build()
            .map_err(|_| Error::new("send_failed", "Cannot create the OTLP HTTP client"))?;
        Ok(Self {
            client,
            base: base.trim_end_matches('/').to_owned(),
            requests: AtomicU64::new(0),
            retries: AtomicU64::new(0),
            bytes_sent: AtomicU64::new(0),
            rejected: Default::default(),
        })
    }

    pub fn from_env(base: &str) -> Result<Self, Error> {
        let headers = match std::env::var(HEADERS_ENV) {
            Ok(raw) => parse_headers(&raw)?,
            Err(std::env::VarError::NotPresent) => HeaderMap::new(),
            Err(_) => return Err(Error::new("invalid_headers", "Headers must be UTF-8")),
        };
        Self::new(base, headers)
    }

    /// Post one gzip-compressed export request, retrying 429, 502, 503, 504,
    /// and transport errors. `Retry-After` seconds are honored up to 60 s.
    pub async fn send(&self, signal: Signal, body: Vec<u8>, format: Format) -> Result<(), Error> {
        let url = endpoint(&self.base, signal);
        let mut delay = Duration::from_millis(250);
        for attempt in 1..=MAX_ATTEMPTS {
            if attempt > 1 {
                self.retries.fetch_add(1, Ordering::Relaxed);
            }
            self.requests.fetch_add(1, Ordering::Relaxed);
            let response = self
                .client
                .post(&url)
                .header(CONTENT_TYPE, format.content_type())
                .header(CONTENT_ENCODING, "gzip")
                .body(body.clone())
                .send()
                .await;
            let wait = match response {
                Ok(response) if response.status().is_success() => {
                    self.bytes_sent.fetch_add(body.len() as u64, Ordering::Relaxed);
                    let bytes = capped_body(response).await?;
                    self.rejected[signal_slot(signal)]
                        .fetch_add(rejected(signal, format, &bytes), Ordering::Relaxed);
                    return Ok(());
                }
                Ok(response) if matches!(response.status().as_u16(), 429 | 502 | 503 | 504) => {
                    retry_after(response.headers()).unwrap_or(delay)
                }
                Ok(response) => {
                    return Err(Error::new(
                        "send_failed",
                        format!(
                            "The OTLP endpoint returned HTTP {} for {}",
                            response.status().as_u16(),
                            signal.name()
                        ),
                    ));
                }
                Err(_) => delay,
            };
            if attempt < MAX_ATTEMPTS {
                tokio::time::sleep(wait).await;
                delay = (delay * 2).min(Duration::from_secs(8));
            }
        }
        Err(Error::new(
            "send_unavailable",
            format!(
                "The OTLP endpoint did not accept {} after {MAX_ATTEMPTS} attempts",
                signal.name()
            ),
        ))
    }

    pub fn summary(&self) -> SendSummary {
        let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
        SendSummary {
            endpoint: self.base.clone(),
            requests: load(&self.requests),
            retries: load(&self.retries),
            bytes_sent: load(&self.bytes_sent),
            rejected_spans: load(&self.rejected[0]),
            rejected_logs: load(&self.rejected[1]),
            rejected_data_points: load(&self.rejected[2]),
        }
    }
}

/// Read a success body of at most [`MAX_RESPONSE_BYTES`]. A larger body is an
/// error; it is rejected from `Content-Length` when present, and otherwise
/// while it streams, so it is never buffered whole.
async fn capped_body(mut response: reqwest::Response) -> Result<Vec<u8>, Error> {
    let too_large = || {
        Error::new(
            "send_failed",
            format!("The OTLP endpoint response is larger than {MAX_RESPONSE_BYTES} bytes"),
        )
    };
    if response.content_length().is_some_and(|length| length > MAX_RESPONSE_BYTES as u64) {
        return Err(too_large());
    }
    let mut body = Vec::new();
    // A read error after a success status leaves the request accepted; only
    // the optional partial-success count is lost.
    while let Ok(Some(chunk)) = response.chunk().await {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn signal_slot(signal: Signal) -> usize {
    match signal {
        Signal::Traces => 0,
        Signal::Logs => 1,
        Signal::Metrics => 2,
    }
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let seconds = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()?;
    Some(Duration::from_secs(seconds).min(MAX_RETRY_AFTER))
}

/// Read the OTLP partial-success count from a response body.
fn rejected(signal: Signal, format: Format, body: &[u8]) -> u64 {
    if body.is_empty() {
        return 0;
    }
    let count = match format {
        Format::Protobuf => match signal {
            Signal::Traces => ExportTraceServiceResponse::decode(body)
                .ok()
                .and_then(|r| r.partial_success)
                .map(|p| p.rejected_spans),
            Signal::Logs => ExportLogsServiceResponse::decode(body)
                .ok()
                .and_then(|r| r.partial_success)
                .map(|p| p.rejected_log_records),
            Signal::Metrics => ExportMetricsServiceResponse::decode(body)
                .ok()
                .and_then(|r| r.partial_success)
                .map(|p| p.rejected_data_points),
        },
        Format::Json => {
            let field =
                ["rejectedSpans", "rejectedLogRecords", "rejectedDataPoints"][signal_slot(signal)];
            serde_json::from_slice::<serde_json::Value>(body).ok().and_then(|value| {
                match &value["partialSuccess"][field] {
                    serde_json::Value::String(text) => text.parse().ok(),
                    number => number.as_i64(),
                }
            })
        }
    };
    count.unwrap_or(0).max(0) as u64
}
