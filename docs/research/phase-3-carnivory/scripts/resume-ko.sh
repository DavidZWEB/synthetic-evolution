#!/bin/bash
# Knockout continuation of a saved run: dir name seed ticks knockout-tag retune.json [control]
set -u
D="$(cd "$(dirname "$0")" && pwd)"
dir=$1; name=$2; seed=$3; ticks=$4; tag=$5; retune=$6; control=${7:-scalar}
base="$D/$dir/$name-s$seed-$control"
out="$base.$tag"
[ -f "$out.done" ] && exit 0
"$D/${BIN:-native-gut11}" resume "$base.bundle.json" --ticks "$ticks" --metrics "$out.jsonl" --sample-every 5000 \
  --history "$out.history.jsonl" --history-capacity 65536 --retune "$D/$retune" \
  > "$out.log" 2>&1 \
  && touch "$out.done" || echo "FAILED knockout $name $seed $tag" >> "$D/$dir/failures.txt"
