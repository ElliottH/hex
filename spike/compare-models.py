#!/usr/bin/env python3
"""Run the frozen real corpus on unpatched 0.4.1; all outputs remain private."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import runpy
import subprocess


def digest(path):
    sha = hashlib.sha256()
    with path.open("rb") as file:
        for block in iter(lambda: file.read(1024 * 1024), b""):
            sha.update(block)
    return sha.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--score-only", action="store_true")
    parser.add_argument("--runs", type=int, default=3)
    args = parser.parse_args()
    if args.runs < 1 or (args.prepare_only and args.score_only):
        parser.error("positive runs and mutually exclusive prepare/score modes required")
    os.umask(0o077)
    root = args.corpus.resolve()
    output = root / "model-comparison"
    output.mkdir(exist_ok=True)
    source = root / "results/normalized.manifest.json"
    manifest = json.loads(source.read_text())
    pins_path = Path(__file__).with_name("comparison-models.json")
    pins = json.loads(pins_path.read_text())
    models = Path("target/comparison-models").resolve()
    plan_file = output / "plan.json"
    result_file = output / "results.jsonl"
    if not args.score_only:
        models.mkdir(parents=True, exist_ok=True)
        for clip in manifest["clips"]:
            if digest(source.parent / clip["audio"]) != clip["sha256"]:
                raise ValueError("normalized corpus checksum mismatch")
        paths = {}
        for name, pin in pins.items():
            path = models / pin["filename"]
            # Reuse the already-verified Unified artifact from the boost spike.
            cached = Path("target/boost-model.gguf").resolve()
            if name == "unified" and cached.exists():
                path = cached
            if not path.exists():
                url = f"https://huggingface.co/{pin['repository']}/resolve/{pin['revision']}/{pin['filename']}"
                partial = path.with_suffix(".partial")
                subprocess.run(["curl", "--fail", "--location", "--silent", "--show-error",
                                "--retry", "2", "--max-time", "900", "--output", str(partial), url], check=True)
                if digest(partial) != pin["sha256"]:
                    raise ValueError("downloaded model checksum mismatch")
                partial.rename(path)
            if digest(path) != pin["sha256"]:
                raise ValueError("cached model checksum mismatch")
            paths[name] = str(path)
        # Fixed before inference: names only, no references or error aliases.
        prompt = "Names and technical terms: " + ", ".join(manifest["vocabulary"]) + "."
        arms = [
            dict(name="unified-offline", model="unified", streaming=False),
            dict(name="ultra-offline", model="ultra", streaming=False),
            dict(name="qwen-offline", model="qwen", streaming=False),
            dict(name="qwen-hints-offline", model="qwen", streaming=False, prompt=prompt),
            dict(name="unified-stream", model="unified", streaming=True),
        ]
        plan = dict(manifest=str(source), manifest_sha256=digest(source), models=paths,
                    model_pins=pins, arms=arms, runs=args.runs)
        plan_file.write_text(json.dumps(plan, indent=2) + "\n")
        if args.prepare_only:
            print("Pinned models and private corpus verified; comparison plan ready.")
            return
        with result_file.open("w") as file, (output / "stderr.log").open("w") as log:
            subprocess.run([str(args.binary.resolve()), str(plan_file), str(args.runs)],
                           stdout=file, stderr=log, check=True, timeout=3600)
    plan = json.loads(plan_file.read_text())
    if digest(source) != plan["manifest_sha256"]:
        raise ValueError("reference manifest changed since inference")
    records = [json.loads(line) for line in result_file.read_text().splitlines()]
    metadata = [r for r in records if r["type"] == "model"]
    if len(metadata) != 3 or any(r["engine"] != "0.4.1" for r in metadata):
        raise ValueError("unexpected engine/model metadata")
    scorer = runpy.run_path(str(Path(__file__).with_name("evaluate-real.py")))
    summaries = []
    for arm in plan["arms"]:
        rows = [r for r in records if r.get("arm") == arm["name"]]
        if {r["run"] for r in rows} != set(range(1, plan["runs"] + 1)):
            raise ValueError("missing measured pass")
        cohorts = scorer["score_cohorts"](manifest, rows)
        first = {r["id"]: r["transcript"] for r in rows if r["run"] == 1}
        changes = sum(r["transcript"] != first[r["id"]] for r in rows if r["run"] > 1)
        summaries.append(dict(name=arm["name"], cohorts=cohorts,
                              changed_transcripts_after_first_pass=changes))
    prompt = next(a["prompt"] for a in plan["arms"] if "prompt" in a)
    public = dict(engine="transcribe-cpp 0.4.1 unpatched", model_pins=plan["model_pins"],
                  manifest_sha256=plan["manifest_sha256"], raw_result_sha256=digest(result_file),
                  runs=plan["runs"], prompt_sha256=hashlib.sha256(prompt.encode()).hexdigest(),
                  prompt_terms=len(manifest["vocabulary"]),
                  model_metadata=metadata, results=summaries)
    (output / "summary.json").write_text(json.dumps(public, indent=2) + "\n")
    print("Five-arm comparison complete; aggregate summary saved.")


if __name__ == "__main__":
    main()
