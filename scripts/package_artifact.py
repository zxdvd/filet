#!/usr/bin/env python3
"""Package a native Filet binary and its portable agent resources."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import zipfile


TARGETS = {
    "x86_64-unknown-linux-gnu": "filet-linux-x64",
    "aarch64-apple-darwin": "filet-macos-arm64",
    "x86_64-apple-darwin": "filet-macos-x64",
    "x86_64-pc-windows-msvc": "filet-windows-x64",
}
RESOURCES = ("README.md", "docs", "examples", "schemas", "types", "skills/filet")


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def package(repo, binary, target, revision, output):
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("revision must be a complete lowercase Git commit SHA")
    name = TARGETS[target]
    windows = target.endswith("windows-msvc")
    binary_name = "filet.exe" if windows else "filet"
    version = subprocess.check_output([str(binary), "--version"], text=True).strip()
    if not re.fullmatch(r"filet \S+", version):
        raise ValueError("binary did not report a Filet version")
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="filet-package-") as temporary:
        root = Path(temporary) / name
        root.mkdir()
        shutil.copy2(binary, root / binary_name)
        (root / binary_name).chmod(0o755)
        # Use the tracked resource list, so local state, caches and private files
        # accidentally placed below examples/ never enter a downloadable bundle.
        tracked = subprocess.check_output(
            ["git", "ls-files", "-z", "--", *RESOURCES], cwd=repo
        ).decode().split("\0")
        for relative in filter(None, tracked):
            source = repo / relative
            if source.is_symlink():
                raise ValueError(f"symlink resource is unsupported: {relative}")
            destination = root / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)
            destination.chmod(0o644)
        for required in ("README.md", "skills/filet/SKILL.md", "examples/yaml/filet.yaml"):
            if not (root / required).is_file():
                raise ValueError(f"missing packaged resource: {required}")
        metadata = {
            "formatVersion": 1,
            "version": version.split(" ", 1)[1],
            "target": target,
            "revision": revision,
            "binary": binary_name,
            "binarySha256": sha256(root / binary_name),
        }
        (root / "BUILD-METADATA.json").write_text(
            json.dumps(metadata, indent=2) + "\n", encoding="utf-8"
        )
        archive = output / (name + (".zip" if windows else ".tar.gz"))
        if windows:
            with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as bundle:
                for path in sorted(root.rglob("*")):
                    if path.is_file():
                        bundle.write(path, path.relative_to(root.parent).as_posix())
        else:
            # A tar archive preserves executable permission across artifact ZIPs.
            with tarfile.open(archive, "w:gz") as bundle:
                bundle.add(root, arcname=name)
        (output / "SHA256SUMS").write_text(
            f"{sha256(archive)}  {archive.name}\n", encoding="utf-8"
        )
        return archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--target", required=True, choices=TARGETS)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--output-dir", required=True, type=Path)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    print(package(repo, args.binary.resolve(strict=True), args.target,
                  args.revision, args.output_dir.resolve()))


if __name__ == "__main__":
    main()
