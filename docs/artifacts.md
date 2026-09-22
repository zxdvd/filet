# Downloadable CI builds

The [Build artifacts workflow](https://github.com/zxdvd/filet/actions/workflows/release.yml) runs on every push to `main`, every `v*` tag, and manual **Run workflow** requests. Pull requests run the ordinary CI checks. Packages are kept for 30 days, subject to repository retention policy.

Each platform runs optimized native tests, builds with pinned Rust and `Cargo.lock`, checks the binary, and verifies the extracted package before uploading it. Inspect that platform's job result and the commit SHA when choosing a build.

## Choose a package

| Artifact | Platform | File inside the downloaded artifact |
|---|---|---|
| `filet-macos-arm64` | Apple Silicon Mac | `filet-macos-arm64.tar.gz` |
| `filet-macos-x64` | Intel Mac | `filet-macos-x64.tar.gz` |
| `filet-linux-x64` | Linux x86-64, GNU libc | `filet-linux-x64.tar.gz` |
| `filet-windows-x64` | Windows x86-64 | `filet-windows-x64.zip` |

Open a successful run and download the matching entry from **Artifacts** or the job summary's download link. GitHub requires sign-in to download Actions artifacts. Unzip this outer download to obtain the package and `SHA256SUMS`. On Windows the inner package is another ZIP; on macOS/Linux it is a tar.gz so the executable permission survives the outer artifact ZIP.

Each package includes the executable, `skills/filet`, examples, schemas, API declarations, README, and docs. `BUILD-METADATA.json` records the version, Rust target, full source commit, and binary SHA-256. `SHA256SUMS` checks the compressed package. This provides integrity checks, not a publisher signature.

## Install on macOS

Run `uname -m`: `arm64` selects the Apple Silicon package; `x86_64` selects Intel when running in a native terminal. For an Apple Silicon Mac using a translated Intel terminal, choose the arm64 package for native execution.

After unzipping the downloaded artifact, enter that directory. For Apple Silicon:

```sh
shasum -a 256 -c SHA256SUMS
tar -xzf filet-macos-arm64.tar.gz
mkdir -p "$HOME/.local/bin"
install -m 755 filet-macos-arm64/filet "$HOME/.local/bin/filet"
export PATH="$HOME/.local/bin:$PATH"
filet --version
```

For Intel substitute `filet-macos-x64` in both paths. Add `export PATH="$HOME/.local/bin:$PATH"` to your shell startup file if that directory is not already on PATH. Keep the unpacked folder somewhere stable for its bundled skill and helper, for example under `~/Applications/filet/`; the binary can live separately in `~/.local/bin`.

The executable needs neither Rust nor Node.js installed. The optional launchd plist generator needs Python 3. These CI binaries have no Developer ID signing or Apple notarization; if macOS blocks one, inspect the source/run and use the system's normal approval flow only if you trust the build. Do not disable Gatekeeper globally.

Next follow the [macOS configuration and daemon guide](../skills/filet/references/macos.md#2-建立独立的示例配置). Tell your agent to read the extracted `skills/filet/SKILL.md` and use the extracted `skills/filet/scripts/launch_agent.py` path. Set `--binary` to the installed executable's actual path. Stop an existing daemon before replacing the binary and start it again after validation.

## Linux and Windows

Linux: check `sha256sum -c SHA256SUMS`, extract the tar.gz, and install `filet` in a directory on PATH. This is a native Ubuntu 24.04 GNU libc build, not a static musl binary; compatibility with older distributions must be verified separately.

Windows: compare `(Get-FileHash .\filet-windows-x64.zip -Algorithm SHA256).Hash` with `SHA256SUMS`, then extract the inner ZIP. Run `filet-windows-x64\filet.exe --version` and add its containing directory to PATH if desired.

## Build and publication scope

Runners are Ubuntu 24.04 x64, macOS 14 arm64, macOS 15 Intel, and Windows Server 2022 x64. A successful runner test does not establish every older OS version's compatibility. CI packages remain experimental builds. A tag triggers an artifact build; this workflow does not create a GitHub Release, so downloads expire with their artifact retention period.
