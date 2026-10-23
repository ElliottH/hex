#!/usr/bin/env python3
"""Prepare or verify a patched copy of the checksum-pinned crates.io engine."""
import argparse
import hashlib
import io
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--verify-existing", action="store_true")
args = parser.parse_args()
root = Path(__file__).resolve().parent.parent
target = root / "target/boost-engine"
checksum = "1c6946c7bf90046fc51e5bc25373a4f6270ca10fdcf4f35b1174075e43c535b5"
if target.exists() and not args.verify_existing:
    raise SystemExit(f"{target} exists; reuse it with --verify-existing or remove the generated copy explicitly")
with urllib.request.urlopen("https://static.crates.io/crates/transcribe-cpp-sys/transcribe-cpp-sys-0.2.4.crate", timeout=120) as response:
    data = response.read()
if hashlib.sha256(data).hexdigest() != checksum:
    raise SystemExit("engine archive differs from Hex's Cargo.lock checksum")
with tempfile.TemporaryDirectory(prefix="hex-boost-engine-") as temporary:
    prepared = Path(temporary)
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        for entry in archive:
            parts = Path(entry.name).parts[1:]
            if not entry.isfile() or not parts or ".." in parts:
                continue
            dest = prepared.joinpath(*parts)
            dest.parent.mkdir(parents=True, exist_ok=True)
            with dest.open("wb") as out:
                shutil.copyfileobj(archive.extractfile(entry), out)
    subprocess.run(["patch", "-p1", "-i", str(root / "spike/engine.patch")], cwd=prepared, check=True)
    if args.verify_existing:
        for source in prepared.rglob("*"):
            if source.is_file():
                dest = target / source.relative_to(prepared)
                if not dest.exists() or source.read_bytes() != dest.read_bytes():
                    raise SystemExit(f"generated engine mismatch: {source.relative_to(prepared)}")
        print("Existing engine matches the verified archive plus patch")
    else:
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(prepared, target)
