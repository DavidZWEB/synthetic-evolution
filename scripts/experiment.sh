#!/usr/bin/env bash
# Paired multi-seed experiments for the native shell (spec section 7.9).
#
#   scripts/experiment.sh OUT_DIR TICKS FOUNDERS SAMPLE_EVERY "SEED..." PARAMS.json...
#
# Runs every params file x seed under both controls (scalar and structural null v2), JOBS
# at a time (default 4), recording each run's wall-clock seconds beside its metrics.
# Then writes one unranked summary per params file, as text and JSON.
set -euo pipefail

if [[ $# -lt 6 ]]; then
  sed -n '4,8p' "$0" >&2
  exit 2
fi
out=$1 ticks=$2 founders=$3 every=$4 seeds=$5
shift 5
root=$(cd "$(dirname "$0")/.." && pwd)
native=$root/target/release/native

cargo build --release -p native --manifest-path "$root/Cargo.toml"
mkdir -p "$out"

# NUL-separated triples passed as positional arguments, so params paths may
# contain spaces or quotes.
for params in "$@"; do
  for seed in $seeds; do
    for control in scalar structural-null; do
      printf '%s\0%s\0%s\0' "$params" "$seed" "$control"
    done
  done
done | xargs -0 -n 3 -P "${JOBS:-4}" bash -c '
  native=$1 out=$2 ticks=$3 founders=$4 every=$5 params=$6 seed=$7 control=$8
  name=$(basename "$params" .json)
  run="$out/$name-$seed-$control"
  start=$SECONDS
  "$native" --seed "$seed" --ticks "$ticks" --founders "$founders" \
    --sample-every "$every" --params "$params" --control "$control" \
    --metrics "$run.jsonl" > "$run.log"
  echo $((SECONDS - start)) > "$run.seconds"
  echo "done $name seed=$seed control=$control in $((SECONDS - start))s"
' experiment "$native" "$out" "$ticks" "$founders" "$every"

for params in "$@"; do
  name=$(basename "$params" .json)
  # Seeds are numeric, so `dense` cannot also collect `dense-growth` runs.
  runs=("$out/$name"-[0-9]*-scalar.jsonl "$out/$name"-[0-9]*-structural-null.jsonl)
  "$native" summarize "${runs[@]}" > "$out/$name-summary.txt"
  "$native" summarize --json "${runs[@]}" > "$out/$name-summary.json"
  echo "summary: $out/$name-summary.txt"
done
