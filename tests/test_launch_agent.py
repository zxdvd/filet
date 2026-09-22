"""Exercise the distributable helper without installing a service."""

import os
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "scripts/launch_agent.py"


class LaunchAgentTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve() / 'space & <xml> 中文 "quote" $literal'
        self.root.mkdir()
        self.binary = self.root / "filet"
        self.binary.write_text("this fixture must never execute\n")
        self.binary.chmod(0o755)
        self.config = self.root / "filet.yaml"
        self.config.write_text("schemaVersion: 1\n")

    def render(self, binary=None, config=None):
        return subprocess.run([
            sys.executable, str(SCRIPT),
            "--binary", str(binary or self.binary),
            "--config", str(config or self.config),
            "--data-dir", str(self.root / "state"),
            "--log-dir", str(self.root / "logs"),
        ], capture_output=True, check=False)

    def test_exact_paths_and_no_side_effects(self):
        result = self.render()
        self.assertEqual(result.returncode, 0, result.stderr)
        job = plistlib.loads(result.stdout)
        self.assertEqual(job["ProgramArguments"], [
            str(self.binary), "daemon", "--config", str(self.config),
            "--data-dir", str(self.root / "state"), "--json",
        ])
        self.assertEqual(job["WorkingDirectory"], str(self.root))
        self.assertEqual(job["StandardErrorPath"], str(self.root / "logs/stderr.log"))
        self.assertTrue(job["RunAtLoad"])
        self.assertTrue(job["KeepAlive"])
        self.assertEqual(sorted(p.name for p in self.root.iterdir()), ["filet", "filet.yaml"])
        if sys.platform == "darwin":
            plist = self.root / "job.plist"
            plist.write_bytes(result.stdout)
            subprocess.run(["/usr/bin/plutil", "-lint", str(plist)], check=True)

    def test_invalid_inputs_produce_no_plist(self):
        cases = [(self.root / "missing", self.config), (self.binary, self.root)]
        if os.name != "nt":
            no_exec = self.root / "no-exec"
            no_exec.write_text("not executable")
            no_exec.chmod(0o600)
            cases.append((no_exec, self.config))
        for binary, config in cases:
            with self.subTest(binary=binary, config=config):
                result = self.render(binary=binary, config=config)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, b"")


if __name__ == "__main__":
    unittest.main()
