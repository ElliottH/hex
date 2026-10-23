#!/usr/bin/env python3
"""Choose on tuning clips, then evaluate all clips; summarize.py separates held-out."""
import argparse
import json
import os
from pathlib import Path
import runpy
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary", type=Path)
parser.add_argument("model", type=Path)
parser.add_argument("--corpus", type=Path, default=Path("perf/transcription-corpus"))
parser.add_argument("--scorer", type=Path, default=Path("scripts/vocabulary-eval.py"))
args = parser.parse_args()
os.umask(0o077)
score = runpy.run_path(str(args.scorer))["score"]
manifest = json.loads((args.corpus / "manifest.json").read_text())
vocabulary = (args.corpus / "vocabulary.txt").read_text().splitlines()
if set(vocabulary) != set(manifest["vocabulary"]):
    raise SystemExit("boosted and scored vocabulary differ")
out = args.corpus / "results"
out.mkdir(exist_ok=True)
for c in manifest["clips"]:
    c["audio"] = str((args.corpus / c["audio"]).resolve())
tuning = {**manifest, "clips": [c for c in manifest["clips"] if c["group"] == "target" and not c["id"].endswith("-3")]}
streaming = {**manifest, "clips": [c for c in manifest["clips"] if c["group"] != "ordinary"] +
             [c for c in manifest["clips"] if c["group"] == "ordinary"][:10]}


def run(name, definition, bonus, margin, mode="offline", runs=3):
    source = out / (name + ".manifest.json")
    source.write_text(json.dumps(definition))
    output = out / (name + ".jsonl")
    env = {**os.environ, "HEX_BOOST_VOCABULARY": str((args.corpus / "vocabulary.txt").resolve()),
           "HEX_BOOST_BONUS": str(bonus), "HEX_BOOST_MARGIN": str(margin)}
    with output.open("w") as file:
        subprocess.run([str(args.binary.resolve()), str(args.model.resolve()), str(source.resolve()), mode, str(runs)],
                       env=env, stdout=file, check=True)
    report = score(definition, [json.loads(line) for line in output.read_text().splitlines()])
    result = dict(name=name, bonus=bonus, margin=margin, mode=mode, metrics=report)
    (out / (name + ".summary.json")).write_text(json.dumps(result, indent=2))
    print(json.dumps(result), flush=True)
    return result


results = [run("tune-baseline", tuning, 0, 0)]
for margin in [.5, 1.5, 3.0]:
    for bonus in [.5, 1.5, 3.0]:
        results.append(run(f"tune-b{bonus}-m{margin}", tuning, bonus, margin))
# No guard or held-out quality is consulted when choosing the parameters.
best = max(results, key=lambda r: (r["metrics"]["target"]["term_recall"], -r["bonus"], -r["margin"]))
bonus, margin = best["bonus"], best["margin"]
# If there is no tuning gain, still characterize the strongest candidate.
if bonus == 0:
    bonus, margin = 3, 3
    print("No tuning gain; evaluating the strongest candidate for failure analysis", flush=True)
results += [run("baseline", manifest, 0, 0, runs=5),
            run("candidate", manifest, bonus, margin, runs=5),
            run("baseline-stream", streaming, 0, 0, mode="stream", runs=5),
            run("candidate-stream", streaming, bonus, margin, mode="stream", runs=5)]
(out / "sweep.json").write_text(json.dumps(results, indent=2))
