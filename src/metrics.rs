use crate::{
    config::{Config, Preset},
    model::{Groups, Record, Role, resource, schema_url, scope, text},
    semconv::attributes as attr,
};
use opentelemetry_proto::tonic::{
    collector::metrics::v1::ExportMetricsServiceRequest,
    common::v1::KeyValue,
    metrics::v1::{
        AggregationTemporality, Histogram, HistogramDataPoint, Metric, NumberDataPoint,
        ResourceMetrics, ScopeMetrics, Sum, metric::Data, number_data_point::Value,
    },
};

#[derive(Clone, Copy)]
enum Kind {
    Histogram,
    Sum,
}

type Spec = (&'static str, &'static str, Kind);

const GENAI: &[Spec] = &[
    ("gen_ai.client.inference.duration", "s", Kind::Histogram),
    ("gen_ai.client.inference.usage.input_tokens", "{token}", Kind::Sum),
    ("gen_ai.client.inference.usage.output_tokens", "{token}", Kind::Sum),
    ("gen_ai.client.inference.operation.input_tokens", "{token}", Kind::Histogram),
    ("gen_ai.client.inference.operation.output_tokens", "{token}", Kind::Histogram),
];
const WEB: &[Spec] = &[
    ("http.server.request.duration", "s", Kind::Histogram),
    ("synthetic.request.count", "{request}", Kind::Sum),
];
const GATEWAY: &[Spec] = &[("http.server.request.duration", "s", Kind::Histogram)];
const API: &[Spec] = &[
    ("http.server.request.duration", "s", Kind::Histogram),
    ("db.client.operation.duration", "s", Kind::Histogram),
];
const DURATION_BOUNDS: &[f64] = &[0.01, 0.05, 0.1, 0.5, 1.0, 5.0];

fn specs(preset: Preset, role: Role) -> &'static [Spec] {
    match (preset, role) {
        (Preset::Agent | Preset::Rag, _) => GENAI,
        (Preset::Web, _) => WEB,
        (Preset::WebTopology, Role::Gateway) => GATEWAY,
        (Preset::WebTopology, Role::Primary) => API,
    }
}

#[derive(Default)]
struct Points {
    histograms: Vec<Vec<HistogramDataPoint>>,
    sums: Vec<Vec<NumberDataPoint>>,
}

impl Points {
    fn histogram(&mut self, slot: usize, point: HistogramDataPoint) {
        self.histograms.resize_with(self.histograms.len().max(slot + 1), Vec::new);
        self.histograms[slot].push(point);
    }
    fn sum(&mut self, slot: usize, point: NumberDataPoint) {
        self.sums.resize_with(self.sums.len().max(slot + 1), Vec::new);
        self.sums[slot].push(point);
    }
}

fn take<T>(slots: &mut [Vec<T>], slot: usize) -> Vec<T> {
    slots.get_mut(slot).map(std::mem::take).unwrap_or_default()
}

fn seconds(nanos: u64) -> f64 {
    nanos as f64 / 1_000_000_000.0
}

pub fn generate(config: &Config, start: u64, end: u64) -> ExportMetricsServiceRequest {
    let mut groups: Groups<Points> = Groups::new();
    for index in start..end {
        let record = Record::new(config, index);
        let point = |attributes, value, bounds: &[f64]| {
            histogram(&record, config, attributes, value, bounds)
        };
        match config.preset {
            Preset::Agent | Preset::Rag => {
                let points = groups.get(Role::Primary, record.version);
                let mut attributes = record.inference_attributes();
                // A request that failed after the model call (error_spike)
                // still records its error on the operation's points.
                if record.failed && !record.work_failed {
                    attributes.push(text(attr::ERROR_TYPE, record.error_type));
                }
                points.histogram(
                    0,
                    point(attributes.clone(), seconds(record.work_duration()), DURATION_BOUNDS),
                );
                attributes.push(text(attr::GEN_AI_TOKEN_MODALITY, "text"));
                points.sum(1, number(&record, config, attributes.clone(), record.input_tokens));
                points.sum(2, number(&record, config, attributes.clone(), record.output_tokens));
                points.histogram(
                    3,
                    point(
                        attributes.clone(),
                        record.input_tokens as f64,
                        &[256.0, 1024.0, 4096.0, 16384.0],
                    ),
                );
                points.histogram(
                    4,
                    point(attributes, record.output_tokens as f64, &[64.0, 256.0, 1024.0, 4096.0]),
                );
            }
            Preset::Web => {
                let points = groups.get(Role::Primary, record.version);
                let attributes = record.root_attributes(Preset::Web);
                points.histogram(
                    0,
                    point(attributes.clone(), seconds(record.root_duration()), DURATION_BOUNDS),
                );
                points.sum(1, number(&record, config, attributes, 1));
            }
            Preset::WebTopology => {
                let server = record.root_attributes(Preset::WebTopology);
                let mut gateway = server.clone();
                gateway[1] = text(attr::HTTP_ROUTE, crate::traces::GATEWAY_ROUTE);
                let duration = seconds(record.root_duration());
                groups
                    .get(Role::Gateway, record.version)
                    .histogram(0, point(gateway, duration, DURATION_BOUNDS));
                let points = groups.get(Role::Primary, record.version);
                points.histogram(0, point(server, duration - 0.06, DURATION_BOUNDS));
                let mut database = vec![
                    text(attr::DB_SYSTEM_NAME, "postgresql"),
                    text(attr::DB_OPERATION_NAME, "SELECT"),
                    text(attr::DB_COLLECTION_NAME, "orders"),
                ];
                if record.work_failed {
                    database.push(text(attr::ERROR_TYPE, record.error_type));
                }
                points.histogram(
                    1,
                    point(database, seconds(record.work_duration()), DURATION_BOUNDS),
                );
            }
        }
    }
    let resource_metrics = groups
        .0
        .into_iter()
        .map(|((role, version), mut points)| {
            let metrics = specs(config.preset, role)
                .iter()
                .enumerate()
                .map(|(slot, (name, unit, kind))| match kind {
                    Kind::Histogram => {
                        histogram_metric(name, unit, take(&mut points.histograms, slot))
                    }
                    Kind::Sum => sum_metric(name, unit, take(&mut points.sums, slot)),
                })
                .collect();
            ResourceMetrics {
                resource: Some(resource(config, role, version)),
                schema_url: crate::semconv::attributes::SEMCONV_SCHEMA_URL.into(),
                scope_metrics: vec![ScopeMetrics {
                    scope: Some(scope()),
                    metrics,
                    schema_url: schema_url(config.preset).into(),
                }],
            }
        })
        .collect();
    ExportMetricsServiceRequest { resource_metrics }
}

fn number(
    record: &Record,
    config: &Config,
    attributes: Vec<KeyValue>,
    value: i64,
) -> NumberDataPoint {
    NumberDataPoint {
        attributes,
        start_time_unix_nano: record.start,
        time_unix_nano: record.start + config.interval_ns,
        value: Some(Value::AsInt(value)),
        ..Default::default()
    }
}

fn histogram(
    record: &Record,
    config: &Config,
    attributes: Vec<KeyValue>,
    value: f64,
    bounds: &[f64],
) -> HistogramDataPoint {
    let mut bucket_counts = vec![0; bounds.len() + 1];
    bucket_counts[bounds.partition_point(|bound| *bound < value)] = 1;
    HistogramDataPoint {
        attributes,
        start_time_unix_nano: record.start,
        time_unix_nano: record.start + config.interval_ns,
        count: 1,
        sum: Some(value),
        min: Some(value),
        max: Some(value),
        explicit_bounds: bounds.to_vec(),
        bucket_counts,
        ..Default::default()
    }
}

fn histogram_metric(name: &str, unit: &str, data_points: Vec<HistogramDataPoint>) -> Metric {
    Metric {
        name: name.into(),
        unit: unit.into(),
        data: Some(Data::Histogram(Histogram {
            data_points,
            aggregation_temporality: AggregationTemporality::Delta as i32,
        })),
        ..Default::default()
    }
}

fn sum_metric(name: &str, unit: &str, data_points: Vec<NumberDataPoint>) -> Metric {
    Metric {
        name: name.into(),
        unit: unit.into(),
        data: Some(Data::Sum(Sum {
            data_points,
            aggregation_temporality: AggregationTemporality::Delta as i32,
            is_monotonic: true,
        })),
        ..Default::default()
    }
}
