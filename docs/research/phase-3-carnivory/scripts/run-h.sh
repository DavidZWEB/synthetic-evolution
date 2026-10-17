#!/bin/bash
# One run with founders, a resumable bundle, and the species history (phylogeny).
set -u
D="$(cd "$(dirname "$0")" && pwd)"
name=$1; seed=$2; dir=$3; ticks=$4; founders=$5; control=${6:-scalar}
out="$D/$dir/$name-s$seed-$control"
[ -f "$out.done" ] && exit 0
"$D/${BIN:-native-gut9}" --seed "$seed" --ticks "$ticks" --sample-every 5000 --founders "$founders" \
  --control "$control" --params "$D/$dir/$name.json" --metrics "$out.jsonl" \
  --history "$out.history.jsonl" --history-capacity 65536 \
  --save-run "$out.bundle.json" > "$out.log" 2>&1 \
  && touch "$out.done" || echo "FAILED $name $seed $control" >> "$D/$dir/failures.txt"
