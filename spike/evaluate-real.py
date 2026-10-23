#!/usr/bin/env python3
"""Evaluate a private manifest at the frozen synthetic-selected boost settings.

Run through HiveMind. All normalized audio and per-clip outputs stay beside the
private corpus. Summary JSON contains aggregate metrics only; proxy-reference
WER must not be presented as human-audited accuracy.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import runpy
import subprocess


def cohort_filters(manifest):
    sources = {
        "dictation_microphone_history": ("dictation", "microphone", "history-proxy"),
        "meeting_microphone_final": ("meeting", "microphone", "final-proxy"),
        "meeting_system_final": ("meeting", "system", "final-proxy"),
        "meeting_microphone_rolling": ("meeting", "microphone", "rolling-proxy"),
        "meeting_system_rolling": ("meeting", "system", "rolling-proxy"),
    }
    def provenance(clip):
        return clip["origin"], clip["source"], clip["reference_quality"]

    if any(provenance(c) not in sources.values() for c in manifest["clips"]):
        raise ValueError("unsupported reference provenance")
    cohorts = {name: lambda c, key=key: provenance(c) == key for name, key in sources.items()}
    for diagnostic, field in [("corrected_abattoir", "corrected_entry_ids"),
                              ("known_live_hanks", "known_live_hanks"),
                              ("literal_avatar", "collision_guard")]:
        for name, key in sources.items():
            cohorts[f"{diagnostic}_{name}"] = lambda c, key=key, field=field: provenance(c) == key and bool(c.get(field))
    return cohorts


def score_cohorts(manifest, records):
    score = runpy.run_path(str(Path(__file__).resolve().parent.parent / "scripts/vocabulary-eval.py"))["score"]
    score(manifest, records)  # Validate completeness before scoring subsets.
    result = {}
    for cohort, accepts in cohort_filters(manifest).items():
        subset = {**manifest, "clips": [c for c in manifest["clips"] if accepts(c)]}
        if cohort.startswith("corrected_abattoir_"):
            subset["clips"] = [{**c, "expected_terms": ["Abattoir"]} for c in subset["clips"]]
        ids = {c["id"] for c in subset["clips"]}
        if ids:
            result[cohort] = score(subset, [r for r in records if r.get("id") in ids])["all"]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("model", type=Path)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("--score-only", action="store_true", help="summarize existing runs against their recorded manifest")
    args = parser.parse_args()
    os.umask(0o077)
    root = args.corpus.resolve()
    manifest_file = root / ("results/normalized.manifest.json" if args.score_only else "manifest.json")
    manifest = json.loads(manifest_file.read_text())
    vocabulary = (root / "vocabulary.txt").read_text().splitlines()
    if set(vocabulary) != set(manifest["vocabulary"]):
        raise ValueError("boosted and scored vocabulary differ")
    cohort_filters(manifest)
    output = root / "results"
    normalized = root / "normalized"
    output.mkdir(exist_ok=True)
    normalized.mkdir(exist_ok=True)
    for index, clip in enumerate([] if args.score_only else manifest["clips"]):
        path = (root / clip["audio"]).resolve()
        if hashlib.sha256(path.read_bytes()).hexdigest() != clip["sha256"]:
            raise ValueError("private source checksum mismatch")
        dest = normalized / f"clip-{index:04}.wav"
        subprocess.run(["ffmpeg", "-nostdin", "-hide_banner", "-loglevel", "error", "-y",
                        "-i", str(path), "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le", str(dest)], check=True)
        clip["audio"] = str(dest)
        clip["source_sha256"] = clip["sha256"]
        clip["sha256"] = hashlib.sha256(dest.read_bytes()).hexdigest()
    source = output / "normalized.manifest.json"
    if not args.score_only:
        source.write_text(json.dumps(manifest, indent=2))
    summaries = []
    for mode, runs in [("offline", 3), ("stream", 1)]:
        for label, bonus in [("baseline", 0), ("candidate", 1.5)]:
            name = label + "-" + mode
            env = {**os.environ, "HEX_BOOST_VOCABULARY": str(root / "vocabulary.txt"),
                   "HEX_BOOST_BONUS": str(bonus), "HEX_BOOST_MARGIN": str(bonus)}
            result_path = output / (name + ".jsonl")
            if not args.score_only:
                with result_path.open("w") as file:
                    subprocess.run([str(args.binary.resolve()), str(args.model.resolve()), str(source), mode, str(runs)],
                                   env=env, stdout=file, check=True)
            records = [json.loads(line) for line in result_path.read_text().splitlines()]
            result = dict(name=name, bonus=bonus, margin=bonus, runs=runs,
                          cohorts=score_cohorts(manifest, records))
            result["raw_result_sha256"] = hashlib.sha256(result_path.read_bytes()).hexdigest()
            result["manifest_sha256"] = hashlib.sha256(source.read_bytes()).hexdigest()
            (output / (name + ".summary.json")).write_text(json.dumps(result, indent=2))
            summaries.append(result)
            print(json.dumps(result), flush=True)
    (output / "summary.json").write_text(json.dumps(summaries, indent=2) + "\n")


if __name__ == "__main__":
    main()
