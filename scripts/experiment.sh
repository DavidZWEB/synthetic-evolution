#!/usr/bin/env bash
# Paired multi-seed experiments for the native shell (spec section 7.9).
#
#   scripts/experiment.sh OUT_DIR TICKS FOUNDERS SAMPLE_EVERY "SEED..." PARAMS.json...
#
# Runs every params file x seed under both controls (scalar and structural null), JOBS
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

for params in "$@"; do
  for seed in $seeds; do
    for control in scalar structural-null; do
      printf '%s %s %s\n' "$params" "$seed" "$control"
    done
  done
done | xargs -P "${JOBS:-4}" -L 1 bash -c '
  params=$0 seed=$1 control=$2
  name=$(basename "$params" .json)
  run="'"$out"'/$name-$seed-$control"
  start=$SECONDS
  "'"$native"'" --seed "$seed" --ticks "'"$ticks"'" --founders "'"$founders"'" \
    --sample-every "'"$every"'" --params "$params" --control "$control" \
    --metrics "$run.jsonl" > "$run.log"
  echo $((SECONDS - start)) > "$run.seconds"
  echo "done $name seed=$seed control=$control in $((SECONDS - start))s"
'

for params in "$@"; do
  name=$(basename "$params" .json)
  "$native" summarize "$out/$name"-*.jsonl > "$out/$name-summary.txt"
  "$native" summarize --json "$out/$name"-*.jsonl > "$out/$name-summary.json"
  echo "summary: $out/$name-summary.txt"
done
