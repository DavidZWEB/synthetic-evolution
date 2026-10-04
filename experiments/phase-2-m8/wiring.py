"""Functional wiring of archived representative genomes, by species origin tick.

Usage: python3 wiring.py history.jsonl, where the archive comes from a native run
with `--history history.jsonl --representatives` (history schema 3).

A neuron is *functional* when it lies on an enabled-connection path from an input
(sensor target or oscillator) to an output (effector source). Hidden neurons and
connections off every such path cost metabolism but cannot affect behavior.
"""
import json
import sys
from collections import defaultdict

CHANNELS = {"VisionRay": 4, "Chemo": 3, "Interoception": 1}


def analyse(genes):
    neurons, sensors, effectors, conns = {}, [], [], []
    for gene in genes:
        (kind, v), = gene.items()
        if kind == "Neuron":
            neurons[v["id"]] = v
        elif kind == "Sensor":
            sensors.append(v)
        elif kind == "Effector":
            effectors.append(v)
        elif kind == "Connection":
            conns.append(v)
    inputs = {t for s in sensors for t in s["targets"][: CHANNELS[s["modality"]]]}
    clocks = {i for i, n in neurons.items() if n["activation"] == "Oscillator"}
    outputs = {e["source"] for e in effectors}
    enabled = [c for c in conns if c["enabled"]]
    fwd, back = defaultdict(set), defaultdict(set)
    for c in enabled:
        fwd[c["from"]].add(c["to"])
        back[c["to"]].add(c["from"])

    def reach(starts, edges):
        seen, todo = set(starts), list(starts)
        while todo:
            for n in edges[todo.pop()]:
                if n not in seen:
                    seen.add(n)
                    todo.append(n)
        return seen

    from_inputs = reach(inputs | clocks, fwd)
    to_outputs = reach(outputs, back)
    functional = from_inputs & to_outputs
    hidden = set(neurons) - inputs - outputs - clocks
    live_edges = [c for c in enabled if c["from"] in functional and c["to"] in functional]
    sensor_live = sum(
        1 for s in sensors
        if set(s["targets"][: CHANNELS[s["modality"]]]) & functional
    )
    return {
        "genes": len(genes),
        "neurons": len(neurons),
        "hidden": len(hidden),
        "hidden_functional": len(hidden & functional),
        "clocks": len(clocks),
        "clocks_functional": len(clocks & functional),
        "connections": len(conns),
        "enabled": len(enabled),
        "functional_connections": len(live_edges),
        "sensors": len(sensors),
        "sensors_functional": sensor_live,
        "outputs_driven": len(outputs & from_inputs),
        "recurrent": sum(1 for c in enabled if c["from"] == c["to"]),
    }


def main(path, bins):
    rows = defaultdict(list)
    for line in open(path):
        r = json.loads(line)
        d = r.get("data", {})
        rep = d.get("representative")
        if r.get("kind") != "event" or not rep or rep.get("status") != "recorded":
            continue
        tick = int(d["tick"])
        b = next(i for i, edge in enumerate(bins) if tick < edge)
        rows[(d["cohort"], b)].append(analyse(rep["genes"]))
    keys = ["genes", "neurons", "hidden", "hidden_functional", "clocks",
            "clocks_functional", "connections", "enabled", "functional_connections",
            "sensors", "sensors_functional", "outputs_driven"]
    print("cohort / origin ticks <bin> : n | " + " ".join(keys))
    for (cohort, b), items in sorted(rows.items()):
        lo = 0 if b == 0 else bins[b - 1]
        means = " ".join(f"{sum(i[k] for i in items) / len(items):.2f}" for k in keys)
        print(f"{cohort:15} {lo:>7}-{bins[b]:<7}: {len(items):4} | {means}")


if __name__ == "__main__":
    edges = [1, 50_000, 100_000, 200_000, 300_000, 400_001]
    main(sys.argv[1], edges)
