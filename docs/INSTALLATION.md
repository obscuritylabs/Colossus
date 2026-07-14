# Installation

Colossus ships as one native Rust executable. It does not require Python at runtime and
does not import Python-era configuration or SQLite state.

## Native Release Archive

Verify the archive against its `.sha256` sidecar before extraction. Checksums protect
transport integrity; use signed offline-bundle verification when publisher authenticity
is required.

macOS and Linux:

```bash
tar -xzf colossus-VERSION-TARGET.tar.gz
./colossus-VERSION-TARGET/install.sh
export PATH="$HOME/.local/bin:$PATH"
colossus --version
```

Windows PowerShell:

```powershell
Expand-Archive colossus-VERSION-TARGET.zip
.\colossus-VERSION-TARGET\install.ps1
$env:PATH = "$HOME\.local\bin;$env:PATH"
colossus.exe --version
```

Use `--prefix PATH` on Unix or `-Prefix PATH` on Windows for another installation
root. Installers use a destination-local temporary file, reject linked package binaries
and linked destination `bin` directories, and make no network request.

## Source Checkout

Install Rust 1.96, then build with the locked workspace:

```bash
cargo build --locked \
  -p colossus-cli --bin colossus
target/debug/colossus --version
```

For a network-isolated checkout whose Cargo cache is already populated, add `--offline`.

## Initialize Fresh State

From the repository you want Colossus to operate on:

```bash
colossus --config .colossus/config.yaml config init
colossus --config .colossus/config.yaml config show
colossus --config .colossus/config.yaml run "offline smoke"
colossus --config .colossus/config.yaml audit verify
```

`config init` refuses to overwrite an existing file. It emits strict YAML with a unique
platform-credential-store identity and a redb path beside the config. The first runtime
open creates or retrieves mandatory journal and signing keys through Keychain, DPAPI, or
Secret Service. Headless deployments may explicitly configure environment key references
instead; plaintext journal fallback is never allowed.

## Development Verification

From the repository root:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --locked --manifest-path fuzz/Cargo.toml --all-targets
```

See [Offline and Airgapped Operation](OFFLINE_AIRGAP.md) for isolated deployment and
[Release Process](RELEASE.md) for native artifact production.
