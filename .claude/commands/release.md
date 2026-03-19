---
description: Build, release toss, and update homebrew tap
argument-hint: <version>
---

# Release toss v$ARGUMENTS

## Context

- Cargo.toml version: `grep '^version' Cargo.toml | head -1`
- Last tag: `git tag --sort=-version:refname | head -1`
- Commits since last release: `git log $(git tag --sort=-version:refname | head -1)..HEAD --oneline`

## Pre-flight

- Abort if `$ARGUMENTS` is empty, tree is dirty, or not on `main`

## Tasks

1. Bump `version` in `Cargo.toml`, run `cargo check` to update lockfile
2. `cargo build --release` — verify with `./target/release/toss --version`
3. `tar -czf toss-v$VERSION-aarch64-apple-darwin.tar.gz -C target/release toss` and compute `shasum -a 256`
4. Commit `Cargo.toml` + `Cargo.lock`, tag `v$VERSION`, push with tags
5. `gh release create v$VERSION *.tar.gz --title "v$VERSION" --generate-notes`
6. Update `/Users/yangym/Documents/GitHub/homebrew-tap/Formula/toss.rb` with new URL + SHA256
7. Commit and push homebrew-tap
8. Clean up local `.tar.gz`
