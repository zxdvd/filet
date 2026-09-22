# Releases and development builds

Use [GitHub Releases](https://github.com/zxdvd/filet/releases/latest) for versioned downloads. Download the package for your platform and `SHA256SUMS` from **Assets**. Public Release downloads do not require GitHub sign-in, have no outer Actions ZIP, and do not expire after 30 days. They remain available unless the release, attachment, or repository is deleted or made inaccessible.

The [Build artifacts workflow](https://github.com/zxdvd/filet/actions/workflows/release.yml) runs on every push to `main`, every `v*` tag, and manual **Run workflow** requests. Pull requests run the ordinary CI checks. Development Actions artifacts are kept for 30 days, subject to repository retention policy. Version tags also publish a GitHub Release after all three native builds pass.

Each platform runs optimized native tests, builds with pinned Rust and `Cargo.lock`, checks the binary, and verifies the extracted package before uploading it. Inspect that platform's job result and the commit SHA when choosing a build.

## Choose a package

| Artifact | Platform | File inside the downloaded artifact |
|---|---|---|
| `filet-macos-arm64` | Apple Silicon Mac | `filet-macos-arm64.tar.gz` |
| `filet-linux-x64` | Linux x86-64, GNU libc | `filet-linux-x64.tar.gz` |
| `filet-windows-x64` | Windows x86-64 | `filet-windows-x64.zip` |

For a development Actions build, open a successful run and download the matching entry from **Artifacts** or the job summary's download link. GitHub requires sign-in to download Actions artifacts. Unzip this outer download to obtain the package and `SHA256SUMS`. On Windows the inner package is another ZIP; on macOS/Linux it is a tar.gz so the executable permission survives the outer artifact ZIP.

Each package includes the executable, `skills/filet`, examples, schemas, API declarations, README, and docs. `BUILD-METADATA.json` records the version, Rust target, full source commit, and binary SHA-256. `SHA256SUMS` checks the compressed package. This provides integrity checks, not a publisher signature.

## Install on macOS

Use the Apple Silicon package on M-series Macs, including when the terminal runs under Rosetta. Intel Mac packages are no longer built.

Put `filet-macos-arm64.tar.gz` and `SHA256SUMS` in the same directory and run (for an Actions download, unzip the outer ZIP first):

```sh
grep '  filet-macos-arm64.tar.gz$' SHA256SUMS | shasum -a 256 -c -
tar -xzf filet-macos-arm64.tar.gz
mkdir -p "$HOME/.local/bin"
install -m 755 filet-macos-arm64/filet "$HOME/.local/bin/filet"
export PATH="$HOME/.local/bin:$PATH"
filet --version
```

Add `export PATH="$HOME/.local/bin:$PATH"` to your shell startup file if that directory is not already on PATH. Keep the unpacked folder somewhere stable for its bundled skill and helper, for example under `~/Applications/filet/`; the binary can live separately in `~/.local/bin`.

The executable needs neither Rust nor Node.js installed. The optional launchd plist generator needs Python 3. These binaries have no Developer ID signing or Apple notarization; if macOS blocks one, inspect the source/run and use the system's normal approval flow only if you trust the build. Do not disable Gatekeeper globally.

Next follow the [macOS configuration and daemon guide](macos.md#2-建立独立的示例配置). Tell your agent to read the extracted `skills/filet/SKILL.md` and use the extracted `scripts/launch_agent.py` path. Set `--binary` to the installed executable's actual path. Stop an existing daemon before replacing the binary and start it again after validation.

## Linux and Windows

Linux: check `sha256sum --ignore-missing -c SHA256SUMS`, extract the tar.gz, and install `filet` in a directory on PATH. This is a native Ubuntu 24.04 GNU libc build, not a static musl binary; compatibility with older distributions must be verified separately.

Windows: compare `(Get-FileHash .\filet-windows-x64.zip -Algorithm SHA256).Hash` with `SHA256SUMS`, then extract the package ZIP. Run `filet-windows-x64\filet.exe --version` and add its containing directory to PATH if desired.

## Build and publication scope

Runners are Ubuntu 24.04 x64, macOS 14 arm64, and Windows Server 2022 x64. A successful runner test does not establish every older OS version's compatibility. Version 0.1 remains experimental, including packages attached to Releases.

## Publishing a release

Set the workspace version in `Cargo.toml`, update `Cargo.lock`, and add `docs/releases/vX.Y.Z.md`. Then choose one trigger:

- Push a `vX.Y.Z` tag matching the workspace version.
- Run **Build artifacts** manually on `main` and enable **publish**.
- Push a commit to `main` whose entire first line is `release: vX.Y.Z`.

Ordinary main pushes and manual runs with **publish** disabled only create development artifacts. A release request validates the version and notes, waits for all three optimized native builds and package checks, verifies each archive checksum and its version/source metadata, and publishes three packages plus a combined `SHA256SUMS`. The workflow uses its scoped GitHub token; no personal access token is needed. Only the publish job has `contents: write`.

The CLI creates a draft, uploads all attachments, then publishes it. A new tag is created at the exact build commit when releasing from `main`. Existing releases and attachments are never overwritten. If upload fails and leaves a draft, inspect and remove that incomplete draft before retrying. Do not remove a published release to retry a build; use a new version.
