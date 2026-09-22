#!/usr/bin/env python3
"""Verify and run the exact distribution that CI will upload."""

import argparse
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import zipfile

from package_artifact import TARGETS, sha256


def verify(directory, target, revision):
    name = TARGETS[target]
    archive_name = name + (".zip" if target.endswith("windows-msvc") else ".tar.gz")
    archive = directory / archive_name
    expected = f"{sha256(archive)}  {archive_name}\n"
    if (directory / "SHA256SUMS").read_text(encoding="utf-8") != expected:
        raise ValueError("archive checksum mismatch")
    with tempfile.TemporaryDirectory(prefix="filet-distribution-") as temporary:
        # macOS /var can be a symlink: use its canonical path for Filet fixtures.
        unpacked = Path(temporary).resolve()
        if archive.suffix == ".zip":
            with zipfile.ZipFile(archive) as bundle:
                bundle.extractall(unpacked)
        else:
            with tarfile.open(archive) as bundle:
                bundle.extractall(unpacked, filter="data")
        root = unpacked / name
        metadata = json.loads((root / "BUILD-METADATA.json").read_text(encoding="utf-8"))
        if metadata["revision"] != revision or metadata["target"] != target:
            raise ValueError("wrong target or commit in packaged metadata")
        binary = root / metadata["binary"]
        if sha256(binary) != metadata["binarySha256"]:
            raise ValueError("packaged binary checksum mismatch")
        state = unpacked / "verification-state"
        for arguments in (
            ["check", "-c", "examples/yaml/filet.yaml"],
            ["check", "-c", "examples/js/filet.yaml"],
            ["test", "-c", "examples/yaml/filet.yaml", "examples/yaml/fixtures.json"],
        ):
            subprocess.run([str(binary), *arguments, "--data-dir", str(state), "--json"],
                           cwd=root, check=True)
        print(f"Verified {archive_name}: checksum, metadata, executable, YAML/JS and fixtures")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", required=True, type=Path)
    parser.add_argument("--target", required=True, choices=TARGETS)
    parser.add_argument("--revision", required=True)
    args = parser.parse_args()
    verify(args.directory.resolve(), args.target, args.revision)


if __name__ == "__main__":
    main()
