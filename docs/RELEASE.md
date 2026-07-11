# Release Process

Rust alpha versions use `0.6.0-alpha.N`; P0+P1 cutover is `0.6.0`. Release only from a
clean tree after the feature inventory and acceptance matrix accurately describe any
remaining gap.

## Readiness

1. Update the workspace version, `CHANGELOG.md`, user docs, security notes, and migration
   guidance together.
2. Confirm `Cargo.lock` and `fuzz/Cargo.lock` contain only reviewed dependency changes.
3. Run the authoritative gates from `rust/`:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --locked --manifest-path fuzz/Cargo.toml --all-targets
cargo deny --locked check -A license-not-encountered licenses sources bans
cargo deny --locked check -D warnings advisories
cargo audit -D warnings --file Cargo.lock
```

Apply the equivalent deny/audit checks to `fuzz/Cargo.toml` and `fuzz/Cargo.lock`.
Pinned nightly CI runs bounded libFuzzer mutation targets in addition to committed corpus
tests.

## Native Artifacts

The `rust-release-smoke` matrix builds six targets:

- macOS arm64 and x64;
- static Linux musl arm64 and x64;
- Windows arm64 and x64.

Each native job:

1. builds with `--locked --release`;
2. executes version, strict config, credential-free echo, and encrypted audit checks;
3. verifies static linkage for Linux;
4. packages `colossus`/`colossus.exe`, the platform installer, license, and Rust README;
5. writes a SHA-256 sidecar;
6. extracts the completed archive into a clean directory;
7. installs into a clean prefix and repeats version/echo/audit using only the installed
   executable;
8. uploads that target independently.

Do not infer six-target readiness from a host-only build. Every matrix job must be green.

## Security And Live Matrices

Release evidence also includes:

- native Seatbelt/Landlock escape acceptance on macOS/Linux arm64/x64;
- Windows named-pipe authentication and fail-closed sandbox acceptance;
- Docker and Podman OCI isolation/cleanup tests with preloaded digest-pinned images;
- OPA decision, outage, readiness, disclosure, masking, and mTLS tests;
- current/previous pinned Chroma v2 lifecycle tests;
- production and fuzz dependency/license/advisory policy.

Opt-in external tests are not replaced by a skipped local test. Preserve their CI links
with the release record.

## Offline And Signed Distribution

Checksums detect corruption but do not authenticate a publisher. A trusted offline
release includes a signed bundle manifest, immutable hashes, SBOM, publisher key
identity, native archive, and any reviewed workflows, skills, packs, OPA bundles, local
model assets, or MCP servers.

Verify without network:

```bash
colossus --config .colossus/config.yaml bundle verify ./bundle
```

The release operator can materialize a deterministic signed bundle from a reviewed
staging tree and install its current-target executable without an archive tool:

```bash
colossus --config .colossus/config.yaml --approval-mode ask bundle build \
  ./bundle-stage ./bundle --name colossus-offline --version 0.6.0-alpha.N \
  --publisher colossus --created-at CREATED_AT --source-revision GIT_COMMIT \
  --signing-key-reference env:COLOSSUS_BUNDLE_SIGNING_SEED
colossus --config .colossus/config.yaml --approval-mode ask bundle install \
  ./bundle --prefix "$HOME/.local"
```

Retain bundle verification, installed-binary smoke, audit verification, secure-anchor
status, and artifact hashes as release evidence.

## Tag And Publish

After all required CI and acceptance evidence is green:

```bash
git tag -a v0.6.0-alpha.N -m "Colossus v0.6.0-alpha.N"
git push origin v0.6.0-alpha.N
```

Attach all six archives, sidecars, SBOM/signature material, changelog excerpt, and known
limitations. Confirm installation on a clean matching host before announcing the release.

The frozen Python tag/branch is not rebuilt or republished as part of the Rust release.
