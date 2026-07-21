# Task specifications

These files define the JSON responses accepted by lumen-cut's background AI
tasks. They are runtime assets embedded into the application at compile time.

The Rust validators in `src-tauri/src/agent/task.rs` are authoritative. A
worker should return only the requested JSON (or NDJSON for chapters), with no
Markdown fence or explanatory prose.
