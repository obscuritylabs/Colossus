# Release Process

Rust 0.6.0 is the P0+P1 cutover release. Release only from a clean tree after the feature
inventory and acceptance matrix accurately describe any remaining gap.

## Actions Cost Policy

An ordinary push to `main` runs only the Ubuntu quick gate: commit-message validation,
formatting, and locked production/fuzz compilation. Pull requests and merge-queue commits
run the full test, native sandbox/runtime, fuzz, supply-chain, Chroma, and live-security
matrices through the fail-closed `rust-pr-gate`. They do not build release archives.

The six-target artifact/install/bundle matrix and fail-closed `rust-cutover-gate` run only
after an explicit manual dispatch. This avoids repeating costly macOS release builds after
every pull request merge while retaining complete release evidence. From a clean final
commit on `main`, start that validation with:

```bash
gh workflow run ci.yml --ref main
```

A green pull-request gate is required for merge, but it does not replace the green manual
cutover gate required before tagging.

## Readiness

1. Update the workspace version, `CHANGELOG.md`, user docs, security notes, and migration
   guidance together.
2. Confirm `Cargo.lock` and `fuzz/Cargo.lock` contain only reviewed dependency changes.
3. Run the authoritative gates from the repository root:

```bash
./release/verify-local-cutover.sh
```

The verifier requires Rust 1.96.0, `cargo-deny 0.20.2`, and `cargo-audit 0.22.2`.
It resolves Cargo-installed tools from `CARGO_HOME/bin` even when that directory is not
on `PATH`, rejects a reintroduced Python package or tracked Python source, and runs the
production and independent fuzz dependency policies. Its expanded command sequence is:

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo check --locked --manifest-path fuzz/Cargo.toml --all-targets
cargo deny --locked check -A license-not-encountered licenses sources bans
cargo deny --locked check -D warnings advisories
cargo audit -D warnings --file Cargo.lock
```

Apply the equivalent deny/audit checks to `fuzz/Cargo.toml` and `fuzz/Cargo.lock`.
Pinned nightly CI runs bounded libFuzzer mutation targets in addition to committed corpus
tests.

This host-side verifier does not replace the supported-platform, live-service, or bounded
nightly fuzz matrices described below.

## Native Artifacts

The `rust-release-smoke` matrix builds six targets:

- macOS arm64 and x64;
- static Linux musl arm64 and x64;
- Windows arm64 and x64.

Each native job:

1. builds with `--locked --release`;
2. executes version, strict config, credential-free echo, and encrypted audit checks;
3. verifies static linkage for Linux;
4. packages `colossus`/`colossus.exe`, the platform installer, license, and root README;
5. writes a SHA-256 sidecar;
6. extracts the completed archive into a clean directory;
7. installs into a clean prefix and repeats version/echo/audit using only the installed
   executable;
8. uploads that target independently.

Do not infer six-target readiness from a host-only build. Every matrix job must be green.

## Security And Live Matrices

Release evidence also includes:

- native Seatbelt/Landlock escape acceptance on macOS/Linux arm64/x64;
- Windows named-pipe authentication and AppContainer/Job Object sandbox acceptance;
- Docker and Podman OCI isolation/cleanup tests with preloaded digest-pinned images;
- OPA decision, outage, readiness, disclosure, masking, and mTLS tests;
- current/previous pinned Chroma v2 lifecycle tests;
- production and fuzz dependency/license/advisory policy.

Opt-in external tests are not replaced by a skipped local test. Preserve their CI links
with the release record.

The `rust-cutover-gate` job is the required aggregate result. It runs even after a failed,
cancelled, or skipped dependency and succeeds only when the workspace, portability, native
sandbox, Windows runtime, fuzz, supply-chain, six-target release, Chroma, and live-security
jobs all report success in the same workflow run.

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
  ./bundle-stage ./bundle --name colossus-offline --version 0.6.0 \
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
git tag -a v0.6.0 -m "Colossus v0.6.0"
git push origin v0.6.0
```

Attach all six archives, sidecars, SBOM/signature material, changelog excerpt, and known
limitations. Confirm installation on a clean matching host before announcing the release.

The frozen Python tag/branch is not rebuilt or republished as part of the Rust release.
