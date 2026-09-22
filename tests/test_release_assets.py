"""Reject incomplete, corrupt or mixed-revision Release attachments."""

import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from assemble_release import assemble
from package_artifact import TARGETS, sha256


class ReleaseAssetsTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.artifacts = self.root / "artifacts"
        self.output = self.root / "release"
        self.revision = "a" * 40
        for target, name in TARGETS.items():
            self.write_package(target, name)

    def write_package(self, build_target, name, **overrides):
        metadata = dict(target=build_target, revision=self.revision, version="0.1.0")
        metadata.update(overrides)
        data = json.dumps(metadata).encode()
        directory = self.artifacts / name
        directory.mkdir(parents=True, exist_ok=True)
        member = name + "/BUILD-METADATA.json"
        if build_target.endswith("windows-msvc"):
            archive = directory / (name + ".zip")
            with zipfile.ZipFile(archive, "w") as bundle:
                bundle.writestr(member, data)
        else:
            archive = directory / (name + ".tar.gz")
            with tarfile.open(archive, "w:gz") as bundle:
                info = tarfile.TarInfo(member)
                info.size = len(data)
                bundle.addfile(info, io.BytesIO(data))
        (directory / "SHA256SUMS").write_text(f"{sha256(archive)}  {archive.name}\n")
        return archive

    def run_assemble(self):
        assemble(self.artifacts, self.output, "v0.1.0", self.revision)

    def test_complete_set_and_combined_checksums(self):
        self.run_assemble()
        self.assertEqual(len(list(self.output.iterdir())), 4)
        for line in (self.output / "SHA256SUMS").read_text().splitlines():
            digest, name = line.split("  ")
            self.assertEqual(sha256(self.output / name), digest)

    def test_missing_or_corrupt_package_leaves_no_output(self):
        target, name = next(iter(TARGETS.items()))
        archive = self.write_package(target, name)
        archive.write_bytes(archive.read_bytes() + b"corruption")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            self.run_assemble()
        self.assertFalse(self.output.exists())
        archive.unlink()
        with self.assertRaises(FileNotFoundError):
            self.run_assemble()
        self.assertFalse(self.output.exists())

    def test_mixed_version_revision_or_target_leaves_no_output(self):
        target, name = next(iter(TARGETS.items()))
        for field, value in (("version", "0.2.0"), ("revision", "b" * 40),
                             ("target", "wrong-target")):
            with self.subTest(field=field):
                self.write_package(target, name, **{field: value})
                with self.assertRaisesRegex(ValueError, "wrong target, source commit or version"):
                    self.run_assemble()
                self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
