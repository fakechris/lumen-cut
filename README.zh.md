# lumen-cut

[![Release](https://img.shields.io/github/v/release/fakechris/lumen-cut)](https://github.com/fakechris/lumen-cut/releases)
[![Platform](https://img.shields.io/badge/platform-macOS%2014%2B%20%7C%20Windows%2010%2F11-blue)](#环境要求)
[![License: AGPL--3.0](https://img.shields.io/badge/license-AGPL--3.0-blue)](./LICENSE)

[English](README.md) | 中文

lumen-cut 是一个开源桌面编辑器，把口播音频和视频变成可编辑的转写稿、字幕、译文和成片导出。基于 Rust、Tauri 2、React 和 TypeScript 构建。

**快速开始**：从 [Releases](https://github.com/fakechris/lumen-cut/releases)（当前最新 v0.3.1）下载 DMG 或安装包，确保 `ffmpeg` / `ffprobe` 在 `PATH` 上，然后拖入一个媒体文件即可 —— 本地 ASR 模型可在设置里一键安装。也可以用下面的 CLI 无界面驱动整条管线。

## 功能

- 通过选择器或拖放导入本地音频/视频，下载媒体 URL，或在桌面应用里录制麦克风。
- 在设置中准备、选择并校验本地 Qwen3-ASR 与词级对齐模型。
- 跟踪长转写进度，安全取消，中断后可重试。
- 编辑、拆分、合并、隐藏、搜索、替换字幕条。
- 识别、预览、指派、重命名、重识别、合并说话人，并附带带时间戳的媒体证据。
- 通过 OpenAI 兼容或 Anthropic API 做翻译、润色、标点修复、章节生成、B-roll 建议。
- 管理 B-roll 建议与本地素材，并在剪辑中预览。
- 在可拖动的媒体时间线上审阅和还原可逆的语音清理剪辑。
- 保存项目版本与分支，带恢复快照。
- 运行交付检查，从桌面应用导出 SRT、VTT、ASS、Markdown、渲染视频或可编辑的 Final Cut Pro 时间线。
- 桌面应用、`lumen-cut-cli`、本地 MCP/HTTP 任务接口三种入口都可用。

CLI 还暴露面向自动化的 audit、task、MCP 和 HTTP 接口。桌面功能都有可发现的 UI 路径、进度和恢复状态；底层自动化接口留给高级用户。

单命令 CLI 示例：

```bash
# 仅 ASR（默认）
lumen-cut-cli auto talk.mp4 --source-lang en --out ./projects

# 转写 → 翻译 → 对齐（跳过润色）
lumen-cut-cli auto talk.mp4 --source-lang en --lang zh --no-polish --out ./projects

# 软剪检测 / 列表 / 还原
lumen-cut-cli cut ./projects/talk --auto
lumen-cut-cli cut ./projects/talk --list --kind filler
lumen-cut-cli export ./projects/talk --srt --bilingual --lang zh -o talk.zh.srt
lumen-cut-cli align list talk --lang zh --fit 16 --root ./projects
lumen-cut-cli task start align talk --lang zh --groups g1,g2 --align-fit 16 --align-local --root ./projects
lumen-cut-cli task start translate talk --lang zh --second-look semantic --root ./projects

# 软剪检测参数 + 导出时间段
lumen-cut-cli cut ./projects/talk --auto --min-pause 1.0 --compress-to 0.5
lumen-cut-cli export ./projects/talk --srt --start 10 --end 90 -o clip.srt

# 保留 claim/submit HTTP 端点给外部 worker
lumen-cut-cli task serve translate talk --lang zh --root ./projects --port 0
# Worker：GET http://127.0.0.1:<port>/agent/next
#         POST http://127.0.0.1:<port>/agent/submit  { "lease_id", "answer": { "text": "..." } }

# 说话人：指派 / 审阅提案 / 应用
lumen-cut-cli speakers ./projects/talk assign --speaker Host --paragraph 1
lumen-cut-cli speakers ./projects/talk reidentify --review
lumen-cut-cli speakers ./projects/talk proposals
lumen-cut-cli speakers ./projects/talk apply

# 解析项目路径 / 深链 URL（可选 --desktop 把任务排给桌面应用）
lumen-cut-cli project open talk --root ./projects
# URL 形式：lumencut://project/talk  （桌面应用也接受 #project=talk）
```

## 环境要求

- macOS 14 或更新（Apple silicon），或 Windows 10/11 x64
- `PATH` 上有 `ffmpeg` 和 `ffprobe`
- [`uv`](https://docs.astral.sh/uv/)，用于一键本地转写环境安装
- 导入媒体 URL 时需要 `yt-dlp`

应用会在其状态目录下创建隔离的 Python 3.12 运行时（macOS 是 `~/.lumen-cut/runtime`，Windows 是 `%LOCALAPPDATA%\lumen-cut\runtime`），并把选中的模型文件下载到 Hugging Face 缓存。两者都不进本仓库。其他 Lumen 应用已装好的 Qwen3-ASR 权重（例如 lumen-asr 的 `~/Library/Application Support/LumenAsr/models` 目录或已有的 Hugging Face 缓存快照）会通过共享的 [`lumen-models`](https://github.com/fakechris/lumen-suite) crate 发现并复用，不会重复下载。Node.js 20+ 与 Rust stable 仅开发时需要。

### Windows 说明

本地转写跑在 Apple MLX 上，仅限 macOS。Windows 上请在 **Settings → Speech & models** 选择 OpenAI 兼容引擎；管线的其余部分 —— 剪辑、字幕、翻译、B-roll、导出 —— 都是原生的。硬件视频编码在 GPU 支持时用 NVENC、Quick Sync 或 AMF，否则回退 `libx264`。安装包目前未签名，首次运行 SmartScreen 会告警。详见 [docs/WINDOWS_PORT_STATUS.md](docs/WINDOWS_PORT_STATUS.md)。

## 开发

```bash
pnpm install
pnpm tauri dev
```

构建前端并跑完整 Rust 测试套件：

```bash
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

创建并打包所有本地发布产物：

```bash
pnpm release:local
```

原始 Tauri 输出在 `src-tauri/target/release/`。可安装的 bundle 在 `src-tauri/target/release/bundle/`。打包脚本把可分发的 DMG、压缩的 app、CLI 归档和 SHA-256 校验和收集到顶层 `build/` 目录。

GitHub Actions 对 push 和 pull request 跑同样的检查和打包流程。分支与 PR 构建使用 ad-hoc macOS 签名。版本 tag 需要 `APPLE_CERTIFICATE`、`APPLE_CERTIFICATE_PASSWORD`、`KEYCHAIN_PASSWORD`、`APPLE_ID`、`APPLE_PASSWORD`、`APPLE_TEAM_ID` 这些仓库 secrets；workflow 导入 Developer ID 证书，对 app 和 DMG 做公证并 staple，用 Gatekeeper 校验，然后创建 GitHub Release 并附上 `build/` 里的全部文件。任何凭据缺失时，tag 构建会直接失败，绝不发布未签名版本。

## 项目结构

```text
src/             React 桌面界面
src-tauri/       Rust 应用、CLI、管线、导出与测试
sidecars/        ASR 与说话人分离的 Python 入口
task-specs/      后台 AI 任务的 JSON 响应规范
scripts/         本地发布打包助手
```

项目数据存放在 `~/Library/Application Support/lumen-cut/Projects/<project-id>/`。原始媒体文件原地引用，删除项目时永不删除。

## AI 配置

核心转写与字幕编辑不需要 API key。可选的翻译与增强任务可使用 OpenAI 兼容或 Anthropic 端点，在桌面设置界面配置。本地任务服务器与 worker 池在需要时自动启动。

## 安全与隐私

- 任务与 MCP HTTP 服务只绑定 loopback。
- 媒体访问限定于桌面应用当前打开的项目。
- 云端 AI 任务只把任务载荷发给用户选择的端点。
- 模型权重、API key、录音、项目数据、私有评测材料一律不入源码管理。

安全问题请私下报告给项目维护者，不要开公开 issue。见 [SECURITY.md](SECURITY.md)。欢迎按 [CONTRIBUTING.md](CONTRIBUTING.md) 的指引参与贡献。

## 许可证

本项目基于 GNU Affero General Public License v3 或更新版本授权。见 [LICENSE](LICENSE)。
