# lumen-cut

lumen-cut is an open-source desktop editor for turning spoken audio and video
into editable transcripts, subtitles, translations, and finished exports.
It is built with Rust, Tauri 2, React, and TypeScript.

## Features

- Import local audio/video, download supported media URLs, or record a microphone.
- Transcribe locally with Qwen3-ASR and optional word-level alignment.
- Edit, split, merge, hide, search, and replace subtitle cues.
- Identify, rename, and merge speakers.
- Translate, polish, repair punctuation, generate chapters, and suggest B-roll
  through an OpenAI-compatible or Anthropic API.
- Review reversible speech-cleanup cuts on a media timeline.
- Export SRT, VTT, ASS, Markdown, rendered video, and FCPXML.
- Use the desktop app, `lumen-cut-cli`, or the local MCP/HTTP task interfaces.

## Requirements

- macOS 14 or newer on Apple silicon
- Node.js 20 or newer
- Rust stable
- `ffmpeg` and `ffprobe` on `PATH`
- Python 3.10 or newer for the local ASR and diarization sidecars
- `yt-dlp` for URL imports

Model files are downloaded separately and are not stored in this repository.

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
requests. Pushing a version tag such as `v0.1.0` creates a GitHub Release and
attaches every file from `build/`.

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
