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
uv run colossus packs list
uv run colossus packs show opensearch
uv run colossus packs verify ./pack
uv run colossus packs validate ./pack
uv run colossus packs install ./pack --allow-untrusted
uv run colossus packs disable demo-pack
uv run colossus packs enable demo-pack
uv run colossus packs uninstall demo-pack
uv run colossus packs trust list
uv run colossus packs trust add colossus
```

Unsigned or untrusted external packs are blocked by default. Use
`--allow-untrusted` only for local development or reviewed internal packs.

Inside the REPL, use `/packs list`, `/packs show NAME`, `/packs verify SOURCE`,
`/packs validate SOURCE`, `/packs install SOURCE`, `/packs enable NAME`,
`/packs disable NAME`, and `/packs trust ...`.

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
  "capabilities": ["integrations", "skills", "tools", "binaries", "docker", "tests"],
  "permissions": ["network"],
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
  "skills": [{"path": "skills/demo-skill"}],
  "tools": [
    {
      "name": "demo.tool",
      "command": "bin/demo-tool",
      "args": [],
      "env_refs": {},
      "permissions": ["network"]
    }
  ],
  "mcp_servers": [],
  "binaries": ["bin/demo-tool"],
  "docker": ["docker/Dockerfile"],
  "docs": ["docs/README.md"],
  "tests": ["tests/test_demo.md"]
}
```

Every listed file must stay inside the pack directory, be a regular file, match the
declared size, and match the declared SHA-256 digest.

Pack validation also checks that referenced integrations, skills, docs, tests, Docker
assets, and binaries are hash-listed; nested skill files are declared; executable tools
declare permissions; and command paths point to declared files.

## Skills In Packs

Pack skills use the same layouts as standalone skills:

- `SKILL.md` with Agent Skills frontmatter.
- `manifest.json` plus `SKILL.md`.
- Optional `references/`, `scripts/`, `assets/`, `examples/`, and `tests/`.

Use `uv run colossus skills new NAME --pack ./pack --resources references,scripts` to
scaffold a pack skill under `./pack/skills/NAME`. Add the generated files to the pack
manifest before `packs validate`.

## Local OCI Layouts

V1 supports installing from local OCI-layout artifacts. The first supported layer must
contain a pack directory with `colossus.pack.json`.

Remote registry pull, push, auth, and hosted registry workflows are deferred. The local
OCI shape exists so future registry support can use the same artifact format.
