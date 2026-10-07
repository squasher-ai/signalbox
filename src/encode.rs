use crate::{config::Format, error::Error};
use prost::Message;
use serde::Serialize;
use serde_json::{Map, Value};

/// Encode an OTLP export request as either wire-format protobuf or OTLP/JSON.
///
/// `opentelemetry-proto` leaves a few metrics `fixed64` fields
/// without custom serde serializers. The OTLP/JSON mapping requires every
/// 64-bit integer to be represented by a decimal string, so the JSON branch
/// applies that mapping after the generated serde implementation runs.
pub fn encode<T>(message: &T, format: Format) -> Result<Vec<u8>, Error>
where
    T: Message + Serialize,
{
    match format {
        Format::Protobuf => Ok(message.encode_to_vec()),
        Format::Json => {
            let mut value = serde_json::to_value(message)
                .map_err(|error| Error::new("json_encode_failed", error.to_string()))?;
            normalize_int64_fields(&mut value);
            serde_json::to_vec(&value)
                .map_err(|error| Error::new("json_encode_failed", error.to_string()))
        }
    }
}

/// Names of OTLP protobuf `int64`, `uint64`, `sfixed64`, and `fixed64` fields.
/// Generated serde handles trace/log timestamps and AnyValue.intValue, but not
/// all metric fields. Keeping this allowlist avoids changing ordinary doubles,
/// enums, and counts that are only 32-bit protocol fields.
const INT64_FIELDS: &[&str] = &[
    "startTimeUnixNano",
    "timeUnixNano",
    "endTimeUnixNano",
    "observedTimeUnixNano",
    "intValue",
    "asInt",
    "count",
    "zeroCount",
    "bucketCounts",
    "rejectedSpans",
    "rejectedLogRecords",
    "rejectedDataPoints",
];

fn normalize_int64_fields(value: &mut Value) {
    match value {
        Value::Object(map) => normalize_object(map),
        Value::Array(items) => {
            for item in items {
                normalize_int64_fields(item);
            }
        }
        _ => {}
    }
}

fn normalize_object(map: &mut Map<String, Value>) {
    for (key, value) in map.iter_mut() {
        normalize_int64_fields(value);
        if INT64_FIELDS.contains(&key.as_str()) {
            normalize_numbers(value);
        }
    }
    // Generated serde nests Exemplar's oneof under "value". In ProtoJSON,
    // oneof fields appear directly on the containing message.
    if map.contains_key("filteredAttributes")
        && let Some(Value::Object(oneof)) = map.remove("value")
    {
        map.extend(oneof);
    }
    // ProtoJSON emits absent optional messages and values by omission.
    map.retain(|_, value| !value.is_null());
}

fn normalize_numbers(value: &mut Value) {
    match value {
        Value::Number(number) => *value = Value::String(number.to_string()),
        Value::Array(items) => {
            for item in items {
                normalize_numbers(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::{
        collector::metrics::v1::ExportMetricsServiceRequest,
        common::v1::InstrumentationScope,
        metrics::v1::{
            AggregationTemporality, Exemplar, Histogram, HistogramDataPoint, Metric,
            NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum, exemplar, metric::Data,
            number_data_point,
        },
        resource::v1::Resource,
    };

    fn histogram_request() -> ExportMetricsServiceRequest {
        ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: Some(Resource::default()),
                scope_metrics: vec![ScopeMetrics {
                    scope: Some(InstrumentationScope::default()),
                    metrics: vec![
                        Metric {
                            name: "request.duration".into(),
                            unit: "ms".into(),
                            data: Some(Data::Histogram(Histogram {
                                data_points: vec![HistogramDataPoint {
                                    start_time_unix_nano: 1_000,
                                    time_unix_nano: 2_000,
                                    count: 7,
                                    sum: Some(9.25),
                                    bucket_counts: vec![2, 5],
                                    explicit_bounds: vec![1.0],
                                    exemplars: vec![Exemplar {
                                        time_unix_nano: 1_500,
                                        value: Some(exemplar::Value::AsInt(9_007_199_254_740_993)),
                                        ..Default::default()
                                    }],
                                    ..Default::default()
                                }],
                                aggregation_temporality: AggregationTemporality::Cumulative as i32,
                            })),
                            ..Default::default()
                        },
                        Metric {
                            name: "active.requests".into(),
                            unit: "1".into(),
                            data: Some(Data::Sum(Sum {
                                data_points: vec![NumberDataPoint {
                                    start_time_unix_nano: 1_000,
                                    time_unix_nano: 2_000,
                                    value: Some(number_data_point::Value::AsInt(
                                        -9_007_199_254_740_993,
                                    )),
                                    ..Default::default()
                                }],
                                is_monotonic: true,
                                aggregation_temporality: AggregationTemporality::Delta as i32,
                            })),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
    }

    #[test]
    fn json_preserves_otlp_numbers_and_oneof_encoding() {
        let bytes = encode(&histogram_request(), Format::Json).unwrap();
        let actual: Value = serde_json::from_slice(&bytes).unwrap();
        let metrics = &actual["resourceMetrics"][0]["scopeMetrics"][0]["metrics"];
        let histogram = &metrics[0]["histogram"];
        let point = &histogram["dataPoints"][0];
        let expected = serde_json::json!({
            "startTimeUnixNano": "1000", "timeUnixNano": "2000",
            "count": "7", "bucketCounts": ["2", "5"],
            "sum": 9.25, "explicitBounds": [1.0], "flags": 0,
        });
        for (key, value) in expected.as_object().unwrap() {
            assert_eq!(&point[key], value, "incorrect OTLP field: {key}");
        }
        assert_eq!(histogram["aggregationTemporality"], 2);
        assert!(point.get("min").is_none());
        let exemplar = &point["exemplars"][0];
        assert_eq!(exemplar["asInt"], "9007199254740993");
        assert_eq!(exemplar["timeUnixNano"], "1500");
        assert!(exemplar.get("value").is_none());
        let sum = &metrics[1]["sum"];
        assert_eq!(sum["dataPoints"][0]["asInt"], "-9007199254740993");
        assert_eq!(sum["aggregationTemporality"], 1);
        assert_eq!(sum["isMonotonic"], true);
    }

    #[test]
    fn protobuf_round_trip_preserves_the_request() {
        let request = histogram_request();
        let bytes = encode(&request, Format::Protobuf).unwrap();
        let decoded = ExportMetricsServiceRequest::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded, request);
    }
}
