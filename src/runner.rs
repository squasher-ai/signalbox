use crate::{
    config::{Config, Progress, Signal},
    encode::encode,
    error::Error,
    sink::Sink,
};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::IsTerminal,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};
use tokio::task::JoinSet;

#[derive(Debug, Serialize, Deserialize)]
pub struct Summary {
    pub schema_version: u8,
    pub status: String,
    pub output: String,
    pub records: u64,
    pub shards_per_signal: u64,
    pub spans: u64,
    pub logs: u64,
    pub metric_points: u64,
    pub bytes_written: u64,
    pub elapsed_ms: u64,
    pub records_per_second: u64,
    /// Scenarios labeled with each configured fault, in `faults` order.
    pub affected_records: Vec<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send: Option<crate::send::SendSummary>,
}

pub fn plan(config: &Config) -> Value {
    json!({ "schema_version": 1, "status": "planned", "config": config,
        "shards_per_signal": config.shards(), "counts": counts_json(config),
        "max_in_flight_records": config.jobs * config.batch_size,
        "files": format!("{{signal}}-{{shard:012}}.otlp.{}", config.format.extension()),
        "identity": identity(config), "send": send_plan(config),
        "fault_windows": config.faults.iter().map(|fault| {
            let (first, end) = crate::faults::window(config, fault);
            json!({ "kind": fault.kind, "first_record": first, "end_record_exclusive": end })
        }).collect::<Vec<_>>(),
        "semantic_conventions": conventions() })
}

fn identity(config: &Config) -> Value {
    json!({ "run_id": config.run_id,
        "id_salt": format!("{:016x}", crate::model::id_salt(config)) })
}

fn send_plan(config: &Config) -> Value {
    config.send.as_ref().map_or(Value::Null, |base| {
        let urls: Vec<_> =
            config.signals.iter().map(|signal| crate::send::endpoint(base, *signal)).collect();
        json!({ "endpoints": urls, "headers_env": crate::send::HEADERS_ENV,
            "content_encoding": "gzip", "max_concurrent_requests": config.jobs })
    })
}

fn counts_json(config: &Config) -> Value {
    let (spans, logs, metric_points) = counts(config);
    json!({ "spans": spans, "logs": logs, "metric_points": metric_points })
}

fn conventions() -> Value {
    json!({ "core_version": crate::semconv::attributes::SEMCONV_VERSION,
        "core_schema_url": crate::semconv::attributes::SEMCONV_SCHEMA_URL,
        "genai_attribute_commit": crate::semconv::attributes::GENAI_SEMCONV_COMMIT,
        "genai_schema_url": crate::semconv::attributes::GENAI_SEMCONV_SCHEMA_URL,
        "genai_metrics_source_commit": crate::semconv::attributes::GENAI_METRICS_COMMIT,
        "profile": "genai-development-pinned-metrics",
        "genai_stability": "development" })
}

fn counts(config: &Config) -> (u64, u64, u64) {
    (
        if config.signals.contains(&Signal::Traces) {
            config.count * config.preset.spans_per_record()
        } else {
            0
        },
        if config.signals.contains(&Signal::Logs) {
            config.count + crate::faults::total_burst_logs(config)
        } else {
            0
        },
        if config.signals.contains(&Signal::Metrics) {
            config.count * config.preset.points_per_record()
        } else {
            0
        },
    )
}

pub async fn run(config: Config) -> Result<Summary, Error> {
    config.validate()?;
    let sender = match &config.send {
        Some(base) => Some(Arc::new(crate::send::Sender::from_env(base)?)),
        None => None,
    };
    let sink =
        Arc::new(Sink::create(&config.output, config.endpoint.as_deref(), &config.region).await?);
    let started = Instant::now();
    let config = Arc::new(config);
    let cursor = Arc::new(AtomicU64::new(0));
    let progress = progress_bar(&config);
    let mut workers = JoinSet::new();
    for _ in 0..config.jobs.min(config.shards() as usize) {
        let config = config.clone();
        let sink = sink.clone();
        let cursor = cursor.clone();
        let progress = progress.clone();
        let sender = sender.clone();
        workers.spawn(async move {
            let mut bytes_written = 0u64;
            let mut affected = vec![0u64; config.faults.len()];
            loop {
                let shard = cursor.fetch_add(1, Ordering::Relaxed);
                if shard >= config.shards() {
                    break;
                }
                let start = shard * config.batch_size as u64;
                let end = start.saturating_add(config.batch_size as u64).min(config.count);
                if !config.faults.is_empty() {
                    let tally_config = config.clone();
                    let hits = tokio::task::spawn_blocking(move || {
                        crate::faults::tally(&tally_config, start, end)
                    })
                    .await
                    .map_err(|_| Error::new("worker_failed", "Generation worker stopped"))?;
                    affected.iter_mut().zip(hits).for_each(|(total, hits)| *total += hits);
                }
                for signal in &config.signals {
                    let generation_config = config.clone();
                    let signal = *signal;
                    let compress = sender.is_some();
                    let (size, bytes) = tokio::task::spawn_blocking(move || {
                        let bytes = match signal {
                            Signal::Traces => encode(
                                &crate::traces::generate(&generation_config, start, end),
                                generation_config.format,
                            ),
                            Signal::Logs => encode(
                                &crate::logs::generate(&generation_config, start, end),
                                generation_config.format,
                            ),
                            Signal::Metrics => encode(
                                &crate::metrics::generate(&generation_config, start, end),
                                generation_config.format,
                            ),
                        }?;
                        let size = bytes.len() as u64;
                        Ok::<_, Error>((
                            size,
                            if compress { crate::send::gzip(&bytes)? } else { bytes },
                        ))
                    })
                    .await
                    .map_err(|_| Error::new("worker_failed", "Generation worker stopped"))??;
                    bytes_written += size;
                    if let Some(sender) = &sender {
                        sender.send(signal, bytes, config.format).await?;
                        continue;
                    }
                    sink.write(
                        &format!(
                            "{}-{shard:012}.otlp.{}",
                            signal.name(),
                            config.format.extension()
                        ),
                        bytes,
                        config.format.content_type(),
                    )
                    .await?;
                }
                progress.inc(end - start);
            }
            Ok::<_, Error>((bytes_written, affected))
        });
    }
    let mut bytes_written = 0;
    let mut affected = vec![0u64; config.faults.len()];
    while !workers.is_empty() {
        tokio::select! {
            result = workers.join_next() => {
                let (bytes, hits) = result.expect("workers remain").map_err(|_| Error::new("worker_failed", "Output worker stopped"))??;
                bytes_written += bytes;
                affected.iter_mut().zip(hits).for_each(|(total, hits)| *total += hits);
            },
            result = tokio::signal::ctrl_c() => {
                result.map_err(|_| Error::new("signal_failed", "Cannot install interrupt handler"))?;
                progress.abandon_with_message("Interrupted; output is incomplete");
                return Err(Error::new("interrupted", "Generation interrupted. Partial files remain; use a new output target to retry"));
            },
        }
    }
    let (spans, logs, metric_points) = counts(&config);
    let elapsed = started.elapsed();
    let summary = Summary {
        schema_version: 1,
        status: "complete".into(),
        output: config.output.clone(),
        records: config.count,
        shards_per_signal: config.shards(),
        spans,
        logs,
        metric_points,
        bytes_written,
        elapsed_ms: elapsed.as_millis() as u64,
        records_per_second: (config.count as f64 / elapsed.as_secs_f64().max(0.000_001)) as u64,
        affected_records: affected,
        send: sender.as_ref().map(|sender| sender.summary()),
    };
    let ground_truth = crate::ground_truth::build(&config, &summary.affected_records);
    sink.write(
        "ground_truth.json",
        serde_json::to_vec_pretty(&ground_truth)
            .map_err(|_| Error::new("encode_failed", "Cannot encode ground truth"))?,
        "application/json",
    )
    .await?;
    let manifest = json!({ "schema_version": 1, "status": "complete", "config": &*config,
        "semantic_conventions": conventions(), "summary": &summary,
        "identity": identity(&config), "ground_truth": "ground_truth.json",
        "shards_sent": config.send.is_some(),
        "files": { "pattern": format!("{{signal}}-{{shard:012}}.otlp.{}", config.format.extension()),
            "shard_index_start": 0, "shards_per_signal": config.shards(),
            "records_in_last_shard": config.count - (config.shards() - 1) * config.batch_size as u64 } });
    sink.write(
        "manifest.json",
        serde_json::to_vec_pretty(&manifest)
            .map_err(|_| Error::new("encode_failed", "Cannot encode manifest"))?,
        "application/json",
    )
    .await?;
    progress.finish_with_message("Complete");
    Ok(summary)
}

fn progress_bar(config: &Config) -> ProgressBar {
    let visible = matches!(config.progress, Progress::Always)
        || matches!(config.progress, Progress::Auto) && std::io::stderr().is_terminal();
    let progress = ProgressBar::with_draw_target(
        Some(config.count),
        if visible { ProgressDrawTarget::stderr() } else { ProgressDrawTarget::hidden() },
    );
    progress.set_style(ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} scenarios {per_sec} ETA {eta}").expect("static progress template"));
    progress.enable_steady_tick(std::time::Duration::from_millis(100));
    progress
}
