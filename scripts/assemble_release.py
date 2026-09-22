#!/usr/bin/env python3
"""Check all native packages before assembling GitHub Release attachments."""

import argparse
import json
from pathlib import Path
import re
import shutil
import tarfile
import zipfile

from package_artifact import TARGETS, sha256


def assemble(artifacts, output, tag, revision):
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", tag):
        raise ValueError("release tag must be vMAJOR.MINOR.PATCH")
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("revision must be a complete lowercase Git commit SHA")
    packages = []
    checksums = []
    for target, name in TARGETS.items():
        archive_name = name + (".zip" if target.endswith("windows-msvc") else ".tar.gz")
        directory = artifacts / name
        archive = directory / archive_name
        checksum = f"{sha256(archive)}  {archive_name}\n"
        if (directory / "SHA256SUMS").read_text(encoding="utf-8") != checksum:
            raise ValueError(f"checksum mismatch: {name}")
        member = name + "/BUILD-METADATA.json"
        if archive.suffix == ".zip":
            with zipfile.ZipFile(archive) as bundle:
                metadata = json.loads(bundle.read(member))
        else:
            with tarfile.open(archive) as bundle:
                with bundle.extractfile(member) as stream:
                    metadata = json.load(stream)
        if (metadata.get("target"), metadata.get("revision"), metadata.get("version")) != (
            target, revision, tag[1:]
        ):
            raise ValueError(f"wrong target, source commit or version: {name}")
        packages.append(archive)
        checksums.append(checksum)
    # Do not leave a publishable partial set when any package is invalid.
    output.mkdir(parents=True, exist_ok=False)
    for archive in packages:
        shutil.copyfile(archive, output / archive.name)
    (output / "SHA256SUMS").write_text("".join(checksums), encoding="utf-8")
    print(f"Verified {len(packages)} release packages for {tag} at {revision}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--revision", required=True)
    args = parser.parse_args()
    assemble(args.artifacts, args.output_dir, args.tag, args.revision)


if __name__ == "__main__":
    main()
