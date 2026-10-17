#!/bin/bash
# Merge a run's segments (first + r1 [+ r2...]) into long/<tag>.jsonl and .history.jsonl.
# usage: stitch.sh <dir> <name> <seed> <tag>
D="$(cd "$(dirname "$0")" && pwd)"
dir=$1; name=$2; seed=$3; tag=$4
base="$D/$dir/$name-s$seed-scalar"
segs=("$base.jsonl"); hsegs=("$base.history.jsonl")
for r in r1 r2 r3; do
  [ -f "$base.$r.jsonl" ] && segs+=("$base.$r.jsonl") && hsegs+=("$base.$r.history.jsonl")
done
python3 "$D/merge.py" "$D/long/$tag-scalar.jsonl" "${segs[@]}" > /dev/null
python3 "$D/mergeh.py" "$D/long/$tag-scalar.history.jsonl" "${hsegs[@]}" > /dev/null
echo "$tag: ${#segs[@]} segments"
