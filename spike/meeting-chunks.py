#!/usr/bin/env python3
"""Replay sentence-aligned synthetic meeting chunks through offline RNN-T.

This exercises decoder state across multiple sentences, not Hex's scheduler,
pause detector, echo suppression, overlapping seams, or capture devices.
"""
import argparse
import json
import os
from pathlib import Path
import runpy
import subprocess
import wave

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary", type=Path)
parser.add_argument("model", type=Path)
parser.add_argument("--corpus", type=Path, default=Path("perf/transcription-corpus"))
args = parser.parse_args()
os.umask(0o077)
root = Path(__file__).resolve().parent.parent
source = root / "tests/fixtures/vocabulary/manifest.json"
manifest = json.loads(source.read_text())
if set((args.corpus / "vocabulary.txt").read_text().splitlines()) != set(manifest["vocabulary"]):
    raise SystemExit("boosted and scored vocabulary differ")
out = args.corpus / "results"
audio_dir = args.corpus / "meeting-chunks"
audio_dir.mkdir(exist_ok=True)
clips = []


def save(group, members, frames):
    ident = f"meeting-{group}-{len(clips) + 1}"
    path = audio_dir / (ident + ".wav")
    with wave.open(str(path), "wb") as wav:
        wav.setparams((1, 2, 16000, 0, "NONE", "not compressed"))
        wav.writeframes(frames)
    clips.append(dict(id=ident, audio=str(path.resolve()), group=group,
                      expected=" ".join(c["expected"] for c in members),
                      expected_terms=list(dict.fromkeys(t for c in members for t in c.get("expected_terms", []))),
                      source_ids=[c["id"] for c in members]))


for group in ["target", "distractor"]:
    members, frames = [], b""
    for clip in [c for c in manifest["clips"] if c["group"] == group]:
        with wave.open(str(source.parent / clip["audio"]), "rb") as wav:
            assert (wav.getnchannels(), wav.getsampwidth(), wav.getframerate()) == (1, 2, 16000)
            chunk = wav.readframes(wav.getnframes()) + bytes(16000)  # 500 ms pause
        if members and len(frames + chunk) > 20 * 32000:
            save(group, members, frames)
            members, frames = [], b""
        members.append(clip)
        frames += chunk
    if members:
        save(group, members, frames)

definition = dict(vocabulary=manifest["vocabulary"], clips=clips)
manifest_path = out / "meetings.manifest.json"
manifest_path.write_text(json.dumps(definition, indent=2))
candidate = json.loads((out / "candidate.summary.json").read_text())
score = runpy.run_path(str(root / "scripts/vocabulary-eval.py"))["score"]
for name, bonus, margin in [("baseline-meetings", 0, 0),
                            ("candidate-meetings", candidate["bonus"], candidate["margin"])]:
    env = {**os.environ, "HEX_BOOST_VOCABULARY": str((args.corpus / "vocabulary.txt").resolve()),
           "HEX_BOOST_BONUS": str(bonus), "HEX_BOOST_MARGIN": str(margin)}
    output = out / (name + ".jsonl")
    with output.open("w") as file:
        subprocess.run([str(args.binary.resolve()), str(args.model.resolve()),
                        str(manifest_path.resolve()), "offline", "5"], env=env, stdout=file, check=True)
    metrics = score(definition, [json.loads(line) for line in output.read_text().splitlines()])
    result = dict(name=name, bonus=bonus, margin=margin, mode="meeting-chunks", metrics=metrics)
    (out / (name + ".summary.json")).write_text(json.dumps(result, indent=2))
    print(json.dumps(result), flush=True)
