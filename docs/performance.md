# Performance

Squasher Signalbox is designed for fast local fixture generation.
It generates one bounded shard at a time per worker, encodes OTLP protobuf on
Tokio's blocking pool, and writes each shard before taking the next one. The
`--jobs` value controls the number of concurrent workers. Increase it until
the storage device becomes the limit.

Run the reproducible local benchmark after building a release binary:

```sh
cargo build --release --locked
./scripts/bench-local.sh
```

The benchmark uses 100,000 scenarios, 1,000 scenarios per shard, all three
signals, protobuf output, a local temporary directory, and five runs. It does
not contact S3 or an OTLP collector. Override the workload with
`BENCH_COUNT`, `BENCH_BATCH_SIZE`, `BENCH_JOBS`, `BENCH_REPEATS`, or point at
an existing binary with `SIGNALBOX_BIN`.

As a reference, an Apple Silicon development machine produced 260,903,727
bytes with four workers. Five wall-clock runs were 0.41, 0.18, 0.18, 0.19,
and 0.18 seconds; the first run includes filesystem and process warm-up. The
generator summary reported 533,191–612,223 scenarios per second. Results vary
with CPU, filesystem, shard size, and concurrent workloads. Treat these
figures as a reproducibility reference, not a product guarantee.

For comparisons, keep the signal set, payload format, scenario count, shard
size, output device, and worker count fixed. Report both elapsed time and the
generator's `records_per_second` field. JSON output has different costs from
protobuf and should be benchmarked separately.
