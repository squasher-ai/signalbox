use crate::{
    config::{Config, Preset, Progress, Signal},
    faults::FaultPreset,
    runner,
    send::{Sender, parse_headers},
};
use flate2::read::GzDecoder;
use opentelemetry_proto::tonic::collector::{
    logs::v1::ExportLogsServiceRequest, metrics::v1::ExportMetricsServiceRequest,
    trace::v1::ExportTraceServiceRequest,
};
use prost::Message;
use std::{
    io::Read,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

struct Received {
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// A minimal OTLP/HTTP sink: one request per connection. The first
/// `throttle` requests receive 429 with `Retry-After: 0`.
async fn mock_collector(throttle: usize) -> (String, Arc<Mutex<Vec<Received>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let received = Arc::new(Mutex::new(Vec::new()));
    let log = received.clone();
    tokio::spawn(async move {
        let mut served = 0;
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 8192];
            let header_end = loop {
                let read = stream.read(&mut chunk).await.unwrap();
                buffer.extend_from_slice(&chunk[..read]);
                if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                    break end + 4;
                }
                assert!(read > 0, "connection closed before headers");
            };
            let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
            let mut lines = head.split("\r\n");
            let path = lines.next().unwrap().split(' ').nth(1).unwrap().to_owned();
            let headers: Vec<(String, String)> = lines
                .filter_map(|line| line.split_once(':'))
                .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
                .collect();
            let length: usize = headers
                .iter()
                .find(|(name, _)| name == "content-length")
                .map_or(0, |(_, value)| value.parse().unwrap());
            while buffer.len() < header_end + length {
                let read = stream.read(&mut chunk).await.unwrap();
                buffer.extend_from_slice(&chunk[..read]);
            }
            served += 1;
            let response: &[u8] = if served <= throttle {
                b"HTTP/1.1 429 Too Many Requests\r\nretry-after: 0\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            } else {
                log.lock().unwrap().push(Received {
                    path,
                    headers,
                    body: buffer[header_end..header_end + length].to_vec(),
                });
                b"HTTP/1.1 200 OK\r\ncontent-type: application/x-protobuf\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            };
            stream.write_all(response).await.unwrap();
            let _ = stream.shutdown().await;
        }
    });
    (address, received)
}

fn gunzip(body: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::new();
    GzDecoder::new(body).read_to_end(&mut decoded).unwrap();
    decoded
}

#[tokio::test]
async fn send_posts_gzip_otlp_retries_throttling_and_writes_labels() {
    let (address, received) = mock_collector(2).await;
    let temporary = tempfile::tempdir().unwrap();
    let output = temporary.path().join("sent");
    let mut config = Config {
        count: 12,
        batch_size: 5,
        jobs: 2,
        preset: Preset::Web,
        output: output.to_str().unwrap().into(),
        progress: Progress::Never,
        send: Some(format!("{address}/otlp/")),
        ..Config::default()
    };
    crate::faults::apply_presets(&mut config, &[FaultPreset::ReleaseRegression]);
    let summary = runner::run(config).await.unwrap();
    let send = summary.send.as_ref().unwrap();
    assert_eq!(send.retries, 2);
    assert_eq!(send.requests, 9 + 2);
    assert_eq!(send.endpoint, format!("{address}/otlp"));
    let mut files: Vec<_> = std::fs::read_dir(&output)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    assert_eq!(files, ["ground_truth.json", "manifest.json"]);
    let received = received.lock().unwrap();
    assert_eq!(received.len(), 9);
    let (mut spans, mut logs, mut points) = (0, 0, 0);
    for request in received.iter() {
        let header = |name: &str| {
            request.headers.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
        };
        assert_eq!(header("content-encoding"), Some("gzip"));
        assert_eq!(header("content-type"), Some("application/x-protobuf"));
        let body = gunzip(&request.body);
        match request.path.as_str() {
            "/otlp/v1/traces" => {
                spans += ExportTraceServiceRequest::decode(body.as_slice())
                    .unwrap()
                    .resource_spans
                    .iter()
                    .map(|group| group.scope_spans[0].spans.len())
                    .sum::<usize>()
            }
            "/otlp/v1/logs" => {
                logs += ExportLogsServiceRequest::decode(body.as_slice())
                    .unwrap()
                    .resource_logs
                    .iter()
                    .map(|group| group.scope_logs[0].log_records.len())
                    .sum::<usize>()
            }
            "/otlp/v1/metrics" => {
                points += ExportMetricsServiceRequest::decode(body.as_slice())
                    .unwrap()
                    .resource_metrics
                    .len()
            }
            other => panic!("unexpected path {other}"),
        }
    }
    assert_eq!((spans as u64, logs as u64), (summary.spans, summary.logs));
    assert!(points > 0);
}

#[tokio::test]
async fn sender_attaches_configured_headers() {
    let (address, received) = mock_collector(0).await;
    let headers = parse_headers("authorization=Bearer%20token-1, x-team = blue").unwrap();
    let sender = Sender::new(&address, headers).unwrap();
    sender
        .send(Signal::Logs, crate::send::gzip(b"").unwrap(), crate::config::Format::Json)
        .await
        .unwrap();
    let request = &received.lock().unwrap()[0];
    assert!(request.headers.contains(&("authorization".into(), "Bearer token-1".into())));
    assert!(request.headers.contains(&("x-team".into(), "blue".into())));
    assert!(request.headers.contains(&("content-type".into(), "application/json".into())));
}

#[test]
fn header_errors_never_echo_values() {
    for raw in ["authorization", "x-key=bad\nvalue", "content-type=text/plain", "x-key=%zz"] {
        let error = parse_headers(&format!("{raw},x-other=super-secret")).unwrap_err();
        assert!(!error.message.contains("secret") && !error.message.contains("bad"));
        assert_eq!(error.code, "invalid_headers");
    }
    assert_eq!(parse_headers(" a=1 ,, b=x%3Dy ").unwrap().len(), 2);
}
