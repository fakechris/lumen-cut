# lumen-cut

lumen-cut is an open-source desktop editor for turning spoken audio and video
into editable transcripts, subtitles, translations, and finished exports.
It is built with Rust, Tauri 2, React, and TypeScript.

## Features

- Import local audio/video by picker or drag-and-drop, download a media URL,
  or record a microphone in the desktop app.
- Prepare, select, and verify local Qwen3-ASR and word-alignment models from Settings.
- Track long transcription progress, cancel safely, and retry interrupted work.
- Edit, split, merge, hide, search, and replace subtitle cues.
- Identify, preview, assign, rename, reidentify, and merge speakers with timed media evidence.
- Translate, polish, repair punctuation, generate chapters, and suggest B-roll
  through an OpenAI-compatible or Anthropic API.
- Manage B-roll suggestions and local assets, then preview them against the edit.
- Review and restore reversible speech-cleanup cuts on a seekable media timeline.
- Save project versions and branches with recovery snapshots.
- Run delivery checks and export SRT, VTT, ASS, Markdown, rendered video, or
  an editable Final Cut Pro timeline from the desktop app.
- Use the desktop app, `lumen-cut-cli`, or the local MCP/HTTP task interfaces.

The CLI additionally exposes automation-oriented audit, task, MCP, and HTTP
interfaces. Desktop features have discoverable UI paths with progress and
recovery states; lower-level automation remains available to advanced users.

One-shot CLI examples:

```bash
# ASR-only (default)
lumen-cut-cli auto talk.mp4 --source-lang en --out ./projects

# Transcribe → translate → align (skip polish)
lumen-cut-cli auto talk.mp4 --source-lang en --lang zh --no-polish --out ./projects

# Soft-cut detect / list / restore
lumen-cut-cli cut ./projects/talk --auto
lumen-cut-cli cut ./projects/talk --list --kind filler
lumen-cut-cli export ./projects/talk --srt --bilingual --lang zh -o talk.zh.srt
lumen-cut-cli align list talk --lang zh --fit 16 --root ./projects
lumen-cut-cli task start align talk --lang zh --groups g1,g2 --align-fit 16 --root ./projects
```

## Requirements

- macOS 14 or newer on Apple silicon
- `ffmpeg` and `ffprobe` on `PATH`
- [`uv`](https://docs.astral.sh/uv/) for the one-click local transcription setup
- `yt-dlp` when importing media URLs

The app creates an isolated Python 3.12 runtime under `~/.lumen-cut/runtime`
and downloads selected model files into the Hugging Face cache. Neither is
stored in this repository. Node.js 20+ and Rust stable are development-only
requirements.

## Development

```bash
pnpm install
pnpm tauri dev
```

Build the frontend and run the complete Rust test suite:

```bash
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

Create and package all local release artifacts:

```bash
pnpm release:local
```

Raw Tauri output is written under `src-tauri/target/release/`. Installable
bundles are under `src-tauri/target/release/bundle/`. The packaging script
collects the distributable DMG, zipped app, CLI archive, and SHA-256 checksums
in the top-level `build/` directory.

GitHub Actions runs the same checks and packaging process for pushes and pull
requests. Branch and pull-request builds use an ad-hoc macOS signature. Version
tags require `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
`KEYCHAIN_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD`, and `APPLE_TEAM_ID`
repository secrets; the workflow imports the Developer ID certificate,
notarizes and staples both app and DMG, verifies them with Gatekeeper, then
creates the GitHub Release and attaches every file from `build/`. A tag fails
closed instead of publishing an unsigned release when any credential is
missing.

## Project layout

```text
src/             React desktop interface
src-tauri/       Rust application, CLI, pipeline, exports, and tests
sidecars/        Python entry points for ASR and speaker diarization
task-specs/      JSON response specifications for background AI tasks
scripts/         Local release packaging helpers
```

Project data is stored under
`~/Library/Application Support/lumen-cut/Projects/<project-id>/`. Original media
files are referenced in place and are never deleted when a project is removed.

## AI configuration

Core transcription and subtitle editing do not require an API key. Optional
translation and enhancement tasks can use an OpenAI-compatible or Anthropic
endpoint configured in the desktop Settings screen. The local task server and
worker pool start automatically when needed.

## Security and privacy

- Task and MCP HTTP services bind to loopback only.
- Media access is scoped to the project currently open in the desktop app.
- Cloud AI tasks send the task payload to the endpoint selected by the user.
- No model weights, API keys, recordings, project data, or private evaluation
  material belong in source control.

Please report security issues privately to the project maintainers rather than
opening a public issue. See [SECURITY.md](SECURITY.md). Contributions are
welcome under the guidelines in [CONTRIBUTING.md](CONTRIBUTING.md).

## License

This project is licensed under the GNU Affero General Public License,
version 3 or later. See [LICENSE](LICENSE).
