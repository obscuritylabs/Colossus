# Offline and Airgapped Operation

The Rust runtime is offline-first. Its default `echo` provider, embedded redb journal,
built-in policy, Tantivy index, workflows, and repository tools do not require a model
credential or network grant. Journal encryption and checkpoint signing remain mandatory
offline.

## Install A Native Archive

Verify the SHA-256 sidecar before extracting. A checksum detects transfer corruption; it
does not authenticate a publisher, so use `colossus bundle verify` for signed offline
bundles.

macOS and static Linux archives include `install.sh`:

```bash
tar -xzf colossus-VERSION-TARGET.tar.gz
./colossus-VERSION-TARGET/install.sh
$HOME/.local/bin/colossus --version
```

Windows archives include `install.ps1`:

```powershell
Expand-Archive colossus-VERSION-TARGET.zip
.\colossus-VERSION-TARGET\install.ps1
& "$HOME\.local\bin\colossus.exe" --version
```

Pass `--prefix PATH` on Unix or `-Prefix PATH` on Windows to choose another
installation root. Installation uses only files already in the archive, rejects linked
package executables and linked destination `bin` directories, and never invokes Cargo,
Python, or a network client.

A trusted multi-target offline bundle can instead install the exact current target after
signature/hash verification:

```bash
colossus --config .colossus/config.yaml bundle verify ./bundle
colossus --config .colossus/config.yaml --approval-mode ask bundle install \
  ./bundle --prefix "$HOME/.local"
```

Bundle installation is clean-prefix/no-clobber and requires an authorized write root.

## Initialize Offline State

Create fresh Rust YAML and state; Rust does not reuse the frozen Python configuration or
SQLite database:

```bash
mkdir -p .colossus/workflows
colossus --config .colossus/config.yaml config init
colossus --config .colossus/config.yaml config show
colossus --config .colossus/config.yaml run "offline smoke"
colossus --config .colossus/config.yaml audit verify
```

`config init` creates unique key identities and configures the OS Keychain, DPAPI, or
Secret Service provider. No application credential is requested. In headless systems
without an OS credential service, explicitly select the environment key provider in YAML
and inject two independently managed 32-byte key values at process launch. Never write
those values into the YAML file. There is no plaintext journal fallback.

The offline smoke proves strict config parsing, encrypted journal creation, a complete
agent turn through `echo`, durable events, a signed checkpoint, and chain verification.
The sandbox network allowlist is empty by default.

## Offline-Safe Capabilities

These remain useful without network:

- Echo and explicitly installed local model endpoints.
- Filesystem, Git, exact patch, repository-context, and structured process tools within
  configured capability and sandbox grants.
- Sessions, tasks, decisions, plans, goals, subagents, memories, and context compaction.
- Tantivy lexical memory retrieval and deterministic local embeddings.
- Versioned YAML workflows and the authenticated local worker.
- Repository research with deterministic synthesis fallback.
- Skills/resources, signed pack verification, audit views, and signed bundle verification.

Network tools, hosted providers, Chroma, remote OPA, MCP servers, integrations, and
embedding endpoints remain unavailable unless their exact endpoints and credentials are
deliberately provisioned. An absent adapter degrades explicitly; it does not silently
attempt discovery.

## Local Model Endpoint

An OpenAI-compatible server on loopback can be used without internet access. Add a strict
profile and exact origin grant:

```yaml
providers:
  profiles:
    local:
      kind: open_ai_compatible
      model: local-model
      baseUrl: http://127.0.0.1:8000/v1
      credentialReference: null
      timeoutMs: 120000
  roles:
    primary: local

sandbox:
  networkDestinations:
    - http://127.0.0.1:8000
```

Then validate and run:

```bash
colossus --config .colossus/config.yaml provider doctor
colossus --config .colossus/config.yaml run "Reply with exactly: connected"
```

Loopback HTTP is accepted for local development; remote providers require HTTPS and the
configured trust policy.

## Prepare Before Isolation

Collect and review:

- The exact native Colossus archive, SHA-256 sidecar, license, README, and installer.
- A signed offline bundle and publisher keys when authenticity is required.
- The strict YAML configuration with network destinations empty or limited to local
  endpoints.
- Any local model server executable/image and model weights, each with immutable hashes.
- OPA bundles, MCP servers, skills, packs, and workflow definitions required inside the
  boundary.
- An OS credential-store recovery procedure or separately protected environment key
  injection mechanism.
- Exported audit anchors/evidence required by the operating policy.

Keep reviewed artifacts read-only after verification. Do not bring Cargo registries,
Python wheelhouses, or source build tooling unless development inside the airgap is an
explicit requirement.

## Airgap Verification Checklist

Inside the isolated environment:

```bash
colossus --version
colossus --config .colossus/config.yaml config show
colossus --config .colossus/config.yaml policy doctor
colossus --config .colossus/config.yaml state doctor
colossus --config .colossus/config.yaml sandbox doctor
colossus --config .colossus/config.yaml tools list
colossus --config .colossus/config.yaml run "airgap acceptance"
colossus --config .colossus/config.yaml audit verify
colossus --config .colossus/config.yaml audit anchor-status
```

Retain the config hash, archive checksum, signed-bundle verification output, audit
verification, secure-anchor status, and generated run id with the release record.

## Frozen Python Users

Python 0.5 remains frozen at `python-v0.5.0` and on `python-legacy`. Its wheel,
`uv`, SQLite, and JSONL instructions are legacy-only and are intentionally not used by
fresh Rust installations.
