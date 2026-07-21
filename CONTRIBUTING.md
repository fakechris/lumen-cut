# Contributing to lumen-cut

Thanks for helping improve lumen-cut.

## Development setup

Install the requirements listed in the README, then run:

```bash
pnpm install
pnpm tauri dev
```

Before submitting a change, run:

```bash
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
```

Keep changes focused and include regression tests for behavioral fixes. Do not
commit generated builds, model weights, media, project data, credentials, local
evaluation material, vendor research, or content copied from another product.

By contributing, you agree that your contribution is licensed under the
project's AGPL-3.0-or-later license.
