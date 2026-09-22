#!/usr/bin/env python3
"""Render a per-user Filet LaunchAgent. Never write files or start services."""

import argparse
import os
from pathlib import Path
import plistlib
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--data-dir", required=True, type=Path)
    parser.add_argument("--log-dir", required=True, type=Path)
    args = parser.parse_args()
    try:
        binary = args.binary.expanduser().resolve(strict=True)
        config = args.config.expanduser().resolve(strict=True)
        state = args.data_dir.expanduser().resolve()
        logs = args.log_dir.expanduser().resolve()
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise ValueError("--binary must name an executable file")
        if not config.is_file():
            raise ValueError("--config must name a file")
        for directory in (state, logs):
            if directory.exists() and not directory.is_dir():
                raise ValueError("state and log paths must be directories")
        job = {
            "Label": "dev.filet.daemon",
            "ProgramArguments": [
                str(binary), "daemon", "--config", str(config),
                "--data-dir", str(state), "--json",
            ],
            "WorkingDirectory": str(config.parent),
            "EnvironmentVariables": {
                "HOME": str(Path.home()),
                "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
            },
            "RunAtLoad": True,
            "KeepAlive": True,
            "ThrottleInterval": 10,
            "Umask": 0o077,
            "StandardOutPath": str(logs / "stdout.log"),
            "StandardErrorPath": str(logs / "stderr.log"),
        }
        # plistlib handles spaces, Unicode and XML metacharacters without a shell.
        output = plistlib.dumps(job, fmt=plistlib.FMT_XML, sort_keys=False)
    except (OSError, ValueError, RuntimeError) as error:
        parser.error(str(error))
    sys.stdout.buffer.write(output)


if __name__ == "__main__":
    main()
