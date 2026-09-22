# 在 macOS 安装和使用 Filet

目录：[安装](#1-安装) · [首次验证](#2-建立独立的示例配置) · [后台运行](#4-登录后自动运行) · [日常管理](#5-日常管理) · [排错](#6-常见问题)

Filet 自带前台 `daemon`，负责监听、稳定性检查、补扫、计划执行和规则热加载。使用用户级 LaunchAgent 可让它在登录后启动、退出后重新拉起；注销后不会继续运行。不需要 root 或 `sudo`。这是实验版，当前采用源码安装；仓库没有 Homebrew formula 或正式发布的 macOS 安装包。现有 macOS CI 验证的是 Apple Silicon，Intel Mac 原生构建尚未单独验证。

## 1. 安装

先检查 `xcode-select -p`、`rustup --version` 和 `python3 --version`。缺少 Apple 命令行工具时运行以下命令，并等待安装界面完成：

```sh
xcode-select --install
```

缺少 Rust 时，使用 [Rust 官方安装方式](https://rust-lang.org/learn/get-started/)：

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
. "$HOME/.cargo/env"
```

克隆并安装；如果目录已有 checkout，在原目录中检查改动后使用 `git pull --ff-only`，不要覆盖重建。

```sh
mkdir -p "$HOME/src"
git clone https://github.com/zxdvd/filet.git "$HOME/src/filet"
cd "$HOME/src/filet"
cargo install --locked --path crates/filet-cli
filet --version
```

仓库会选择 `rust-toolchain.toml` 固定的 Rust 版本。默认二进制在 `~/.cargo/bin/filet`；自定义了 `CARGO_HOME` 或安装根目录时，以 `command -v filet` 为准。新终端找不到命令时，运行 `. "$HOME/.cargo/env"`。QuickJS 和 SQLite 编译进二进制，运行不需要 Node.js。下面的 plist 生成工具额外需要 Python 3 标准库；先确认 `python3 --version` 可用。

让本机 Agent 读取克隆目录里的 `skills/filet/SKILL.md`。若你的 Agent 支持技能目录，可按它的安装约定复制整个 `skills/filet` 文件夹，保留 `references` 和 `scripts`。直接读取文件也能使用，不要求先安装 ChatGPT 插件。

## 2. 建立独立的示例配置

以下约定让配置、规则、状态和日志都在监听目录之外：

| 用途 | 路径 |
|---|---|
| 配置 | `~/.config/filet/filet.yaml` |
| JS 规则 | `~/.config/filet/rules/` |
| 本地状态 | `~/Library/Application Support/filet/state` |
| 日志 | `~/Library/Logs/filet/` |
| 示例输入 | `~/filet-demo/inbox/` |
| 示例输出 | `~/filet-demo/archive/` |

首次设置时运行：

```sh
mkdir -p "$HOME/.config/filet/rules" "$HOME/filet-demo/inbox"
mkdir -p "$HOME/Library/Application Support/filet/state" "$HOME/Library/Logs/filet"
```

创建 `~/.config/filet/filet.yaml`，内容如下。已有配置时先读取并合并，不要覆盖：

```yaml
schemaVersion: 1
sources:
  inbox:
    path: ~/filet-demo/inbox
    ready:
      stableFor: "3s"
      retryFor: "10m"
rules:
  - id: archive-pdfs
    on:
      type: file.ready
      source: inbox
    when:
      extension: pdf
    actions:
      - move:
          to: ~/filet-demo/archive
          onConflict: error
```

在终端中设置下列变量；新终端需要重新设置。后续所有命令都使用同一个状态目录：

```sh
FILET_CONFIG="$HOME/.config/filet/filet.yaml"
FILET_STATE="$HOME/Library/Application Support/filet/state"
filet check -c "$FILET_CONFIG" --data-dir "$FILET_STATE" --json
filet doctor -c "$FILET_CONFIG" --data-dir "$FILET_STATE" --json
```

放入一份可丢弃的 `sample.pdf`，再查看计划：

```sh
filet plan -c "$FILET_CONFIG" --data-dir "$FILET_STATE" \
  "$HOME/filet-demo/inbox/sample.pdf" --json
```

核对 `data.plan` 的动作和目标路径，然后把输出里的 `data.plan.planId` 填入下方 `PLAN_ID`：

```sh
filet apply -c "$FILET_CONFIG" --data-dir "$FILET_STATE" PLAN_ID --json
filet history --data-dir "$FILET_STATE" --json
```

`plan` 只保存计划；`apply` 才移动文件。首次手动验证时保持 daemon 停止。此示例根据扩展名匹配，不读取 PDF 内容。

## 3. 前台运行

```sh
filet daemon -c "$FILET_CONFIG" --data-dir "$FILET_STATE"
```

它会持续运行并自动执行匹配的动作；按 Ctrl-C 退出。完成最初的目录扫描后再放入新的示例文件。第一次启动的 `file.ready` 会将已有文件作为基线，不会批量处理它们；处理旧文件使用显式 `plan`/`apply`，或有意配置 `scan` 规则。

在准备后台运行前先结束这个前台进程。相同状态目录只允许一个执行实例。

## 4. 登录后自动运行

确认示例已通过、当前配置适合持续自动执行后，在**本机已登录用户的终端**操作。下面假设 checkout 位于 `~/src/filet`；如果使用复制安装的 skill，改成该 skill 的 `scripts/launch_agent.py` 路径。

先生成并检查 plist。生成器只输出文件内容，不创建目录、不安装服务，也不启动 daemon。路径中的空格和 XML 特殊字符会被正确处理。

```sh
mkdir -p "$HOME/Library/LaunchAgents" "$HOME/Library/Logs/filet"
mkdir -p "$HOME/Library/Application Support/filet/state"
filet check -c "$FILET_CONFIG" --data-dir "$FILET_STATE" --json &&
python3 "$HOME/src/filet/skills/filet/scripts/launch_agent.py" \
  --binary "$(command -v filet)" \
  --config "$FILET_CONFIG" \
  --data-dir "$FILET_STATE" \
  --log-dir "$HOME/Library/Logs/filet" \
  > "$HOME/Library/LaunchAgents/dev.filet.daemon.plist.new" &&
plutil -lint "$HOME/Library/LaunchAgents/dev.filet.daemon.plist.new"
```

生成器使用固定服务名 `dev.filet.daemon`，适用于一个用户工作流。首次安装时确认同名 plist 不存在；已安装时先按下一节停止服务，再替换配置。只有上一步验证成功后执行：

```sh
mv "$HOME/Library/LaunchAgents/dev.filet.daemon.plist.new" \
  "$HOME/Library/LaunchAgents/dev.filet.daemon.plist"
chmod 600 "$HOME/Library/LaunchAgents/dev.filet.daemon.plist"
launchctl bootstrap "gui/$(id -u)" "$HOME/Library/LaunchAgents/dev.filet.daemon.plist"
launchctl print "gui/$(id -u)/dev.filet.daemon"
```

这一步会立即开始自动处理，之后每次登录自动运行。`RunAtLoad` 和 `KeepAlive` 控制启动与重启；不要同时用 `nohup`、`&` 或另一个 LaunchAgent 启动相同工作流。启动后查看 stderr 日志并用一个新建的示例文件验证实际效果。

## 5. 日常管理

查看进程、业务状态与日志：

```sh
launchctl print "gui/$(id -u)/dev.filet.daemon"
filet status --data-dir "$FILET_STATE" --json
filet history --data-dir "$FILET_STATE" --json
tail -n 50 "$HOME/Library/Logs/filet/stderr.log"
```

`status` 读取 SQLite 中的历史状态，不能证明 daemon 仍在运行。日志目前不自动轮转。

停止当前会话的服务（plist 保留，下次登录仍会启动）：

```sh
launchctl bootout "gui/$(id -u)/dev.filet.daemon"
```

停止后检查 `launchctl print` 已找不到该服务，再进行独立 `apply`。停止过程中未完成的动作由下次启动恢复；无法确认的动作会进入 `needs_review`，尤其是外部命令，不应盲目重跑。

恢复运行：

```sh
launchctl bootstrap "gui/$(id -u)" "$HOME/Library/LaunchAgents/dev.filet.daemon.plist"
```

升级程序：先停止服务，在 checkout 中运行 `git pull --ff-only` 和 `cargo install --locked --path crates/filet-cli --force`，再执行 `check`、`doctor` 并恢复服务。保留状态目录。替换程序不会让已运行的进程自动升级。

修改规则：先在隔离目录验证，再将完整配置发布到原路径。daemon 约每两秒检查配置和 JS 包变化；无效配置会被拒绝，旧的有效配置继续运行。改变 source 配置会创建新基线；只改规则不会自动重放已观察到的 `file.ready` 文件。需要精确控制启用时间时先停止服务。

取消自动启动：停止服务后删除 `~/Library/LaunchAgents/dev.filet.daemon.plist`。保留配置、状态和处理后的文件；不要为解决锁或恢复问题删除数据库。

## 6. 常见问题

- `INSTANCE_LOCKED`：仍有 daemon 或另一个 apply 使用同一状态目录。找到并停止拥有锁的进程，不要删除锁文件。
- 下载目录权限错误：先检查源目录是否可读。macOS 可能要求在「系统设置 → 隐私与安全性 → 文件与文件夹」中授权相关进程；终端获授权不代表 LaunchAgent 一定获授权。根据实际错误处理，不要用 `sudo` 绕过。可以先用 `~/filet-demo` 验证。
- 找不到外部工具：launchd 不加载交互式 shell 配置。YAML/JS 的 `exec.program` 使用工具的真实绝对路径；Homebrew 的路径可能随 Mac 架构而不同。exec 子进程仍只收到规则显式设置的环境变量。
- `STALE_PLAN` 或类似预检失败：文件或规则发生了变化，重新生成并检查计划。
- 后台服务不断重启：查看 `stderr.log` 中的配置、路径、权限或锁错误；修复后重新加载。使用 `launchctl print` 确认实际 PID 和退出状态。
- Finder 标签等元数据：当前复制不保证保留扩展属性/ACL。对这类工作流先验证结果；不要假定所有 macOS 文件元数据都会保留。

平台参考：[Apple 命令行工具](https://developer.apple.com/documentation/xcode/installing-the-command-line-tools)、[Apple LaunchAgent 文档](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html)、[macOS 文件访问权限](https://support.apple.com/guide/mac-help/mchld5a35146/mac)。具体 `launchctl` 命令也可在本机用 `man launchctl` 核对。
