#!/bin/bash
# A structural-null paired run: metrics only (no history or saved run for this control).
set -u
D="$(cd "$(dirname "$0")" && pwd)"
name=$1; seed=$2; dir=$3; ticks=$4; founders=$5
out="$D/$dir/$name-s$seed-structural-null"
[ -f "$out.done" ] && exit 0
"$D/${BIN:-native-gut15}" --seed "$seed" --ticks "$ticks" --sample-every 5000 --founders "$founders" \
  --control structural-null --params "$D/$dir/$name.json" --metrics "$out.jsonl" > "$out.log" 2>&1 \
  && touch "$out.done" || echo "FAILED null $name $seed" >> "$D/$dir/failures.txt"
