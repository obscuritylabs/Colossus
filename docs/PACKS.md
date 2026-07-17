# Packs

Packs are installable capability packages for Colossus. They are the runtime and
distribution boundary for executable capability. A pack can contribute integrations,
skills, declared executable tools, MCP server declarations, binaries, Docker assets,
docs, and tests without putting vendor-specific code in the core agent.

## Core Model

- Installed packs are available capabilities.
- Connected integrations expose model-callable tools.
- Pack secrets are credential refs, not raw values.
- Pack tools still pass through policy, approval, audit, output limits, and redaction.
- Skills inside packs are still prompt/resource data. Colossus does not execute scripts
  directly from a skill directory.
- Native code is out-of-process through declared MCP servers, not imported into Colossus.
- Every executable or binary file must be hash-listed in `colossus.pack.json` and tied
  to declared permissions.

Bundled first-party packs currently provide GitHub, SearXNG, and OpenSearch
integrations. External packs install into the Colossus data directory.

## Commands

```bash
colossus --config .colossus/config.yaml packs verify ./pack
colossus --config .colossus/config.yaml --approval-mode ask packs install ./pack
colossus --config .colossus/config.yaml packs list
```

The Rust CLI provides
`packs list|show|verify|validate|install|enable|disable|uninstall|call` and
`packs trust list|add`. Lifecycle mutations, executable calls, MCP launches, and trust
additions require approval by default. `packs trust add PUBLISHER --public-key BASE64`
binds the publisher to the SHA-256 identity of an exact Ed25519 public key; trusting a
publisher name by itself is not sufficient.

Unsigned or untrusted external packs are blocked by default. Use
`--allow-untrusted` only for local development or reviewed internal packs.
The override remains an approval-gated effect and does not make the publisher trusted.
If a manifest contains any signature, every signature must be Ed25519, resolve to an
exact publisher/key trust binding, and verify successfully. Invalid or unknown present
signatures fail closed even when `--allow-untrusted` is supplied.

Inside the TUI, use `/packs list`, `/packs show NAME`, `/packs verify SOURCE`,
`/packs validate SOURCE`, `/packs install SOURCE`, `/packs enable NAME`,
`/packs disable NAME`, `/packs uninstall NAME`, `/packs call TOOL`, and
`/packs trust ...`.

## Manifest

Packs use `colossus.pack.json` at the pack root:

```json
{
  "format_version": 1,
  "name": "demo-pack",
  "version": "0.1.0",
  "description": "Demo pack.",
  "publisher": "example",
  "license": "Apache-2.0",
  "homepage": "https://example.com/demo-pack",
  "capabilities": ["integrations", "tools", "binaries"],
  "permissions": ["process", "network"],
  "files": [
    {
      "path": "integrations/demo.json",
      "sha256": "64-character lowercase hex digest",
      "size": 1234,
      "content_type": "application/vnd.colossus.integration+json"
    },
    {
      "path": "bin/demo-tool",
      "sha256": "64-character lowercase hex digest",
      "size": 4096,
      "content_type": "application/octet-stream"
    }
  ],
  "integrations": [{"path": "integrations/demo.json"}],
  "tools": [
    {
      "name": "demo.tool",
      "command": "bin/demo-tool",
      "args": [],
      "env_refs": {},
      "permissions": ["process", "network"]
    }
  ],
  "mcp_servers": [],
  "binaries": ["bin/demo-tool"]
}
```

Every listed file must stay inside the pack directory, be a regular file, match the
declared size, and match the declared SHA-256 digest.

The Rust verifier also rejects absolute or non-normalized paths, every symlink (including
intermediate directory symlinks), special filesystem entries, duplicate declarations,
undeclared payload files, oversized manifests/files/archives, and publisher/signature
mismatches. Signatures cover compact UTF-8 JSON after strict deserialization, default
materialization, recursive lexicographic object-key sorting, and clearing the
`signatures` array. Array order remains significant.

Pack validation also checks that referenced integrations, skills, docs, tests, Docker
assets, and binaries are hash-listed; nested skill files are declared; executable tools
declare permissions; and command paths point to declared files.

The permission vocabulary is `process`, `network`, `filesystem.read`,
`filesystem.write`, and `credentials`. Every tool and MCP server requires `process`;
credential references additionally require `credentials`. A declaration cannot exceed
the pack-level permission ceiling. Built-in policy narrows each pack process permit to
its verified executable and pack root, then adds configured filesystem roots or network
destinations only when that exact declaration requests them. OPA receives pack name,
version, manifest hash, and declared permissions as policy input.

Enabled fixed-argument tools enter the normal model tool registry on the next runtime
start and execute only through the authenticated sandbox helper. Enabled pack MCP
servers enter the configured MCP allowlist with pack-specific effect actions and the
same permission restriction, credential broker, quarantine, post-effect authorization,
and redaction path as first-party MCP configuration.

## Skills In Packs

Pack skills use the same layouts as standalone skills:

- `SKILL.md` with Agent Skills frontmatter.
- `manifest.json` plus `SKILL.md`.
- Optional `references/`, `scripts/`, `assets/`, `examples/`, and `tests/`.

Place the skill under `skills/NAME`, declare that directory in `skills`, and hash-list
every nested regular file before `packs validate`. Pack validation never executes a
skill script; executable behavior must be a declared tool or MCP server instead.

## Local OCI Layouts

V1 supports installing from local OCI-layout artifacts. The first supported layer must
contain a pack directory with `colossus.pack.json`.

The Rust reconstruction accepts verified local directories plus OCI layout 1.0 sources
with one OCI image manifest and a supported tar or tar+gzip pack layer. Descriptor sizes
and SHA-256 digests are checked before bounded extraction. Links, special entries,
duplicate paths, traversal, remote descriptor URLs, archive bombs, and ambiguous layouts
fail closed before the extracted pack enters normal verification.

## Collections And Registry

A collection is a signed `colossus.collection.json` inventory over immediate
`packs/NAME` and `skills/NAME` directories. Builds are reproducible for the same staged
bytes, timestamp, metadata, and signing seed. Every pack retains its own trusted
publisher signature; skills remain data-only; exact pack dependency closure must be
present and acyclic. Install verifies the complete collection, stages every artifact,
refuses existing destinations, publishes pack lifecycle events as one journal batch,
and rolls back synchronous failures.

```bash
colossus collections verify ./collection
colossus --approval-mode ask collections build ./staged ./collection \
  --name starter --version 1.0.0 --publisher example \
  --created-at 2026-07-16T12:00:00Z --signing-key-reference env:COLLECTION_SEED
colossus --approval-mode ask collections install ./collection
```

Registry transport uses the same signed collection as a deterministic tar. Pull writes
only to a clean destination and verifies before publication. Push uses create-only
`PUT` semantics; a conflict counts as replay success only when the server returns the
same `X-Content-Sha256`. URLs require HTTPS except for explicit loopback-IP acceptance,
and optional bearer credentials use `env:VARIABLE` references:

```bash
colossus --approval-mode ask registry pull https://registry.example/v1/starter/1.0.0 ./starter \
  --credential-reference env:REGISTRY_TOKEN
colossus --approval-mode ask registry push ./starter https://registry.example/v1/starter/1.0.0 \
  --credential-reference env:REGISTRY_TOKEN
```

The origin and credential variable must also be granted under `sandbox.networkDestinations`
and `sandbox.environment`. No registry is contacted by default; local verification,
installation, OCI layouts, and offline bundles remain fully offline.
