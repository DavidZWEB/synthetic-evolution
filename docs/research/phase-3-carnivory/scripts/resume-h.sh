#!/bin/bash
# Continue a saved run with metrics and species history: dir name seed ticks [tag] [control]
set -u
D="$(cd "$(dirname "$0")" && pwd)"
dir=$1; name=$2; seed=$3; ticks=$4; tag=${5:-r1}; control=${6:-scalar}
base="$D/$dir/$name-s$seed-$control"
case $tag in r1) prev="$base.bundle.json";; *) prev="$base.r$(( ${tag#r} - 1 )).bundle.json";; esac
out="$base.$tag"
[ -f "$out.done" ] && exit 0
"$D/${BIN:-native-gut11}" resume "$prev" --ticks "$ticks" --metrics "$out.jsonl" --sample-every 5000 \
  --history "$out.history.jsonl" --history-capacity 65536 \
  --save-run "$out.bundle.json" > "$out.log" 2>&1 \
  && touch "$out.done" || echo "FAILED resume $name $seed $tag" >> "$D/$dir/failures.txt"
