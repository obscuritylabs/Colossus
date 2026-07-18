---
title: Extension manifests
description: Declarative skill, pack, collection, integration, and MCP extension formats.
audience: developer
type: reference
---

# Extension manifests

Extensions do not create a second authority path. Skills are data-only; executable
capabilities must be declared by a verified pack, configured MCP server, or supported
integration and still pass access, policy, approval, permit, sandbox, quarantine, and
audit checks.

## Skill directory

```text
skill-name/
  SKILL.md
  manifest.json          # optional
  references/            # optional
  scripts/               # optional, never executed on activation
  assets/                # optional
  examples/              # optional
  tests/                 # optional
```

`SKILL.md` begins with YAML frontmatter containing `name` and `description`, followed by
Markdown instructions. An optional strict `manifest.json` may declare triggers,
required tools, permissions, offline compatibility, and resources. Identity metadata in
both files must agree.

Resource paths are safe relative paths below the five allowed directories. Symlinks,
non-text resources, and oversized content are rejected. Files in `scripts/` are data for
inspection or copying; skill activation never executes them.

## Pack manifest

A pack root contains `colossus.pack.json`:

```json
{
  "format_version": 1,
  "name": "demo-pack",
  "version": "0.1.0",
  "description": "Demo pack.",
  "publisher": "example",
  "license": "Apache-2.0",
  "homepage": "https://example.com/demo-pack",
  "capabilities": ["tools", "binaries"],
  "permissions": ["process", "network"],
  "files": [
    {
      "path": "bin/demo-tool",
      "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
      "size": 4096,
      "content_type": "application/octet-stream"
    }
  ],
  "integrations": [],
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

The permission vocabulary is:

- `process`
- `network`
- `filesystem.read`
- `filesystem.write`
- `credentials`

Every tool and MCP server requires `process`; credential references also require
`credentials`. A child declaration cannot exceed the pack-level permission ceiling.
Every regular payload file is declared with exact path, size, SHA-256, and content type.

Pack verification rejects traversal, absolute or non-normalized paths, links, special
entries, duplicates, undeclared payload, size/hash mismatch, excessive bounds, and
publisher/signature mismatch. Every present signature must resolve to an exact trusted
Ed25519 publisher/key binding and verify.

## Collection manifest

A collection root contains `colossus.collection.json` and immediate
`packs/NAME` and `skills/NAME` directories. Its signed inventory binds:

- collection name, release identifier, publisher, and reproducible creation time;
- every immediate pack and skill artifact;
- exact hashes and sizes;
- pack dependencies and their complete acyclic closure;
- collection Ed25519 signatures.

Every pack retains and verifies its own publisher signature. Skills retain the data-only
contract. Installation re-verifies staged copies and refuses an existing destination.

## Integration records

Built-in integrations are created through typed CLI operations rather than hand-authored
manifest files. Their persisted connection record contains:

- stable connector identity and lifecycle status;
- exact base URL where applicable;
- authentication kind and credential references;
- imported OpenAPI document identity and hash where applicable;
- bounded operation metadata.

Credential values are never persisted. Connected operations are namespaced tools and
remain hidden until selected by access resolution.

JSON OpenAPI 3 imports reject external `$ref`, embedded alternate origins, unsupported
schemas, and unknown arguments. Generated tool names use
`openapi.NAME.OPERATION`.

## MCP server declaration

```yaml
mcp:
  servers:
    local-docs:
      command: /absolute/path/to/mcp-server
      args: [--stdio]
      workingDirectory: /absolute/path/to/repository
      environment:
        API_TOKEN: env:MCP_API_TOKEN
      allowedTools: [search_docs]
      researchTools:
        - tool: search_docs
          title: Internal documentation
          arguments:
            query: "{query}"
      timeoutMs: 30000
      maxOutputBytes: 1048576
```

Servers are stdio-only exact executable identities. Discovery is filtered through
`allowedTools`, and calls are validated against freshly discovered schemas. Each page
and call is a separate effect. Configuration, access, sandbox, and pack permissions may
only narrow the declaration.
