#!/usr/bin/env python3
"""Export aggregate evidence without private paths, audio, or transcript text."""
import argparse
import hashlib
import json
from pathlib import Path
import runpy

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("results", type=Path)
parser.add_argument("output", type=Path)
args = parser.parse_args()
score = runpy.run_path(str(Path(__file__).resolve().parent.parent / "scripts/vocabulary-eval.py"))["score"]
summaries = []
for source in sorted(args.results.glob("*.summary.json")):
    item = json.loads(source.read_text())
    summaries.append({key: item[key] for key in ["name", "bonus", "margin", "mode", "metrics"]})
heldout = {}
for name in ["baseline", "candidate", "baseline-stream", "candidate-stream"]:
    manifest = json.loads((args.results / (name + ".manifest.json")).read_text())
    manifest["clips"] = [c for c in manifest["clips"] if c["group"] == "target" and c["id"].endswith("-3")]
    ids = {c["id"] for c in manifest["clips"]}
    rows = [json.loads(line) for line in (args.results / (name + ".jsonl")).read_text().splitlines()]
    heldout[name] = score(manifest, [r for r in rows if r.get("id") in ids])
hashes = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(args.results.glob("*.jsonl"))}
args.output.write_text(json.dumps(dict(summaries=summaries, heldout=heldout, raw_result_sha256=hashes), indent=2) + "\n")
