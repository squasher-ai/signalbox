use crate::{
    config::{Config, Preset},
    model::{Record, resource, schema_url, scope, text},
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

pub fn generate(config: &Config, start: u64, end: u64) -> ExportMetricsServiceRequest {
    let mut duration = Vec::with_capacity((end - start) as usize);
    let mut input_sum = Vec::with_capacity((end - start) as usize);
    let mut output_sum = Vec::with_capacity((end - start) as usize);
    let mut input_hist = Vec::with_capacity((end - start) as usize);
    let mut output_hist = Vec::with_capacity((end - start) as usize);
    for index in start..end {
        let record = Record::new(config, index);
        let mut attributes = if config.preset == Preset::Web {
            record.root_attributes(config.preset)
        } else {
            record.inference_attributes()
        };
        let elapsed = if config.preset == Preset::Web {
            0.9
        } else {
            record.duration as f64 / 1_000_000_000.0
        };
        duration.push(histogram(
            &record,
            config,
            attributes.clone(),
            elapsed,
            &[0.01, 0.05, 0.1, 0.5, 1.0, 5.0],
        ));
        if config.preset != Preset::Web {
            attributes.push(text(attr::GEN_AI_TOKEN_MODALITY, "text"));
        }
        input_sum.push(number(
            &record,
            config,
            attributes.clone(),
            if config.preset == Preset::Web { 1 } else { record.input_tokens },
        ));
        if config.preset != Preset::Web {
            output_sum.push(number(&record, config, attributes.clone(), record.output_tokens));
            input_hist.push(histogram(
                &record,
                config,
                attributes.clone(),
                record.input_tokens as f64,
                &[256.0, 1024.0, 4096.0, 16384.0],
            ));
            output_hist.push(histogram(
                &record,
                config,
                attributes,
                record.output_tokens as f64,
                &[64.0, 256.0, 1024.0, 4096.0],
            ));
        }
    }
    let metrics = if config.preset == Preset::Web {
        vec![
            histogram_metric("http.server.request.duration", "s", duration),
            sum_metric("synthetic.request.count", "{request}", input_sum),
        ]
    } else {
        vec![
            histogram_metric("gen_ai.client.inference.duration", "s", duration),
            sum_metric("gen_ai.client.inference.usage.input_tokens", "{token}", input_sum),
            sum_metric("gen_ai.client.inference.usage.output_tokens", "{token}", output_sum),
            histogram_metric(
                "gen_ai.client.inference.operation.input_tokens",
                "{token}",
                input_hist,
            ),
            histogram_metric(
                "gen_ai.client.inference.operation.output_tokens",
                "{token}",
                output_hist,
            ),
        ]
    };
    ExportMetricsServiceRequest {
        resource_metrics: vec![ResourceMetrics {
            resource: Some(resource(config)),
            schema_url: crate::semconv::attributes::SEMCONV_SCHEMA_URL.into(),
            scope_metrics: vec![ScopeMetrics {
                scope: Some(scope()),
                metrics,
                schema_url: schema_url(config.preset).into(),
            }],
        }],
    }
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
