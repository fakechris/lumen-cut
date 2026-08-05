# Windows port status

This tracks the deliberate compatibility choices made while making lumen-cut
build and run on Windows 10/11 x64.

## Compatibility policy

- macOS stays a first-class build target. Nothing here degrades it.
- Windows-specific behaviour is target-gated, and every platform difference
  lives behind a named helper rather than an inline `#[cfg]` at the call site.
- The port aims at the same *product*, not a reduced one: cutting, captions,
  translation, B-roll, versions, export and the CLI all work. Only the local
  MLX transcription engine is unavailable, because it does not exist off macOS.

## Implemented

### Paths and application state

- `src-tauri/src/paths.rs` owns every user directory. Windows state lives in
  `%LOCALAPPDATA%\lumen-cut` (settings, logs, the managed Python runtime,
  `setup-job.json`, `pending-open.json`); macOS keeps `~/.lumen-cut`
  unchanged. Projects default to `%LOCALAPPDATA%\lumen-cut\Projects` and stay
  at `~/Library/Application Support/lumen-cut/Projects` on macOS.
- `home_dir()` resolves `HOME` → `USERPROFILE` → `HOMEDRIVE`+`HOMEPATH`, which
  keeps the Hugging Face cache lookup identical to `huggingface_hub`'s own.
- `LUMEN_CUT_STATE_DIR` relocates the whole state root, and is what the tests
  use instead of mutating `HOME`.

### Subprocesses

- Managed children are placed in a Windows **job object** with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, the direct analogue of the Unix
  process group the code already used. Cancelling a job or quitting the app
  tears down ffmpeg's helper processes, not just the direct child.
- Every spawn sets `CREATE_NO_WINDOW`, so console sidecars (ffmpeg, yt-dlp,
  python) never flash a window.
- `proc.rs` tests now run through `/bin/sh` or PowerShell depending on the
  host, so the same contracts are asserted on both platforms.

### Tool and runtime discovery

- `doctor::configure_process_path` seeds `PATH` with the managed venv's
  `Scripts` directory, `%LOCALAPPDATA%\Microsoft\WindowsApps` and the scoop
  shim directory instead of the Homebrew prefixes.
- Python is probed as `python` then the `py` launcher. `python3` is
  deliberately *not* probed on Windows: it is a Microsoft Store alias that
  opens the Store rather than running an interpreter.
- The managed uv virtualenv uses `runtime\Scripts\python.exe`.

### Media

- Microphone capture is resolved by `src-tauri/src/capture.rs`:
  AVFoundation `:0` on macOS, PulseAudio `default` on Linux, and an
  enumerated DirectShow device on Windows. The `@device_cm_{…}` moniker is
  preferred over the friendly name because it is ASCII and survives
  non-English device names. `LUMEN_CUT_AUDIO_INPUT` /
  `LUMEN_CUT_AUDIO_BACKEND` override the choice.
- Hardware video encoding selects the first *working* encoder from
  `h264_nvenc`, `h264_qsv`, `h264_amf` (and the HEVC equivalents) by running a
  throwaway one-frame encode. Listing an encoder in `ffmpeg -encoders` does
  not mean a session can be opened, so a probe is the only honest test. The
  result is cached per process; machines without a supporting GPU fall back to
  `libx264`. Each vendor gets its own rate control (`-cq`, `-global_quality`,
  `-qp_i`/`-qp_p`) since `-crf` is libx26x-only.
- Caption presets map to fonts the host actually ships: `SimSun`/`Consolas` on
  Windows against `Songti SC`/`Menlo` on macOS. The WYSIWYG canvas preview and
  the libass export path use the same table (`src/platform.ts` and
  `src-tauri/src/data/caption_presets.rs`).
- Reveal-in-file-manager uses `explorer.exe /select,` on Windows,
  `open -R` on macOS, `xdg-open` elsewhere. `explorer.exe` returns a non-zero
  exit code even on success, so its status is deliberately ignored.

### UI

- `src/platform.ts` detects the host from the webview user agent. Shortcut
  hints render as `Ctrl+Z` / `Shift+Ctrl+Z` on Windows instead of `⌘Z` / `⇧⌘Z`.
  The key handlers themselves already accepted `metaKey || ctrlKey`.

### Packaging and CI

- `src-tauri/tauri.windows.conf.json` selects the NSIS bundle, a per-user
  install (no elevation), and the WebView2 download bootstrapper.
- `src-tauri/tauri.conf.json` now lists `icon.ico` and the PNG icon set, not
  only `icon.icns`.
- `scripts/windows/collect-release-asset.ps1` is the Windows counterpart of
  `scripts/package-release.sh`: it renames the installer, zips the CLI, and
  writes `SHA256SUMS.txt`.
- `.github/workflows/build-release.yml` gained a `build-windows-x64` job that
  runs the same frontend tests, sidecar tests, `cargo fmt`, Clippy and
  `cargo test` gates as macOS, then builds the installer. The release job now
  waits for both platforms and regenerates one combined `SHA256SUMS.txt`
  (each platform ships its own, which would otherwise collide).

## Known limitations

- **Local transcription is macOS-only.** `mlx-qwen3-asr` is built on Apple
  MLX and has no Windows build. `asr::local_engine_supported()` reports this
  up front and the UI steers to the OpenAI-compatible cloud engine rather than
  offering an install button that cannot succeed. A future CUDA/DirectML
  backend would replace this restriction.
- Speaker diarization (`pyannote.audio` + torch) is CPU-portable and should
  work, but has not been exercised on Windows hardware yet.
- The NSIS installer is unsigned, so SmartScreen warns on first download and
  run. Signing needs a code-signing identity; see lumen-asr's
  `docs/WINDOWS_LOCAL_SIGNING.md` for the approach that repo settled on.
- Only x64 is targeted. Windows on ARM is not built or tested.
- MSI is not produced. NSIS per-user covers the current distribution need.

## Verification

Run on macOS 14 (Apple silicon) against this branch:

- `pnpm test` — 163 passed.
- `pnpm build` — TypeScript and Vite production build passed.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` — clean.
- `cargo test --all-targets` — 517 passed.

Windows verification is **pending** and must be run on a real Windows 10/11
x64 machine. `cargo check --target x86_64-pc-windows-msvc` cannot stand in
from macOS: `ring` (via `rustls`) needs the MSVC toolchain headers to build
its C sources, so cross-checking stops before reaching this crate. GitHub's
`windows-latest` runner is therefore the first authoritative build.

Checklist for the Windows pass:

1. `pnpm install --frozen-lockfile && pnpm test && pnpm build`
2. `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
3. `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
4. `cargo test --manifest-path src-tauri/Cargo.toml --all-targets`
5. `pnpm build:desktop:windows`
6. Install the NSIS output, then confirm end to end: import media, cloud
   transcription, word-level cutting, caption preset rendering, subtitle
   export, video export (verify which encoder was chosen in the logs),
   microphone recording, reveal-in-Explorer, and the diagnostics panel.

Record failures here until they are resolved.
