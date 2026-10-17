#!/bin/bash
# One job: "diag <seed>" (per-species snapshot from the 300k world) or "nobite <seed>" (300k baseline without a bite).
set -u
D="$(cd "$(dirname "$0")" && pwd)"; R="$D/../runs"
kind=$1; s=$2
case $kind in
  diag) "$D/native-brains" resume "$R/seed$s-300k.sevrun" --ticks 20000 --sample-every 5000 \
          --metrics "$D/diag-s$s.jsonl" > "$D/diag-s$s.log" 2>&1 && touch "$D/diag-s$s.done" \
          || echo "FAILED diag $s" >> "$D/failures.txt";;
  nobite) "$D/native-brains" --seed "$s" --ticks 300000 --sample-every 5000 --founders 1000 \
          --params "$D/nobite.json" --metrics "$D/nobite-s$s.jsonl" \
          --history "$D/nobite-s$s.history.jsonl" --history-capacity 65536 \
          --save-run "$D/nobite-s$s.sevrun" > "$D/nobite-s$s.log" 2>&1 && touch "$D/nobite-s$s.done" \
          || echo "FAILED nobite $s" >> "$D/failures.txt";;
esac
