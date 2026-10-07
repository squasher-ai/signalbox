#!/usr/bin/env bash
set -euo pipefail

# Reproducible local protobuf benchmark. The benchmark writes to a temporary
# directory and does not contact S3 or an OTLP collector.
count="${BENCH_COUNT:-100000}"
batch_size="${BENCH_BATCH_SIZE:-1000}"
jobs="${BENCH_JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || printf '4')}"
repeats="${BENCH_REPEATS:-5}"
binary="${SIGNALBOX_BIN:-target/release/squasher-signalbox}"

if [[ ! -x "$binary" ]]; then
  cargo build --release --locked
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/squasher-signalbox-bench.XXXXXX")"
trap 'rm -rf "$work"' EXIT

printf 'binary=%s count=%s batch_size=%s jobs=%s repeats=%s\n' \
  "$binary" "$count" "$batch_size" "$jobs" "$repeats"
for run in $(seq 1 "$repeats"); do
  output="$work/run-$run"
  /usr/bin/time -p "$binary" generate \
    --count "$count" \
    --batch-size "$batch_size" \
    --jobs "$jobs" \
    --signals traces,logs,metrics \
    --format protobuf \
    --output "$output" \
    --progress never \
    >"$work/result-$run.json" \
    2>"$work/time-$run.txt"
  printf 'run=%s ' "$run"
  awk '/^(real|user|sys)/ { printf "%s=%ss ", $1, $2 }' "$work/time-$run.txt"
  printf 'summary='
  cat "$work/result-$run.json"
done
