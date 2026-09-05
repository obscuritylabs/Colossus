# OCI distribution and air gaps

Colossus distributes one whole plugin per OCI artifact. It uses a standard OCI image
manifest with artifact type `application/vnd.colossus.agent-plugin.v1`, config type
`application/vnd.colossus.agent-plugin.config.v1+json`, and one content layer of type
`application/vnd.colossus.agent-plugin.content.v1.tar+gzip`. The archive root is exactly
`<plugin-name>/`. The OCI manifest digest is identity; a mutable tag is only a lookup.

Use Colossus packaging to produce sorted paths, normalized archive metadata, and
deterministic gzip. Do not hand-build archives. The profile rejects links, traversal,
duplicates, special files, and image indexes as plugin payloads. Limits are 1 MiB for
manifests/config, 256 MiB per file, 2 GiB total, and 10,000 files.

```sh
colossus plugins validate ./example-plugin
colossus plugins package ./example-plugin --output ./example-plugin.oci
colossus plugins verify ./example-plugin.oci --trust-profile development
colossus plugins install --layout ./example-plugin.oci --trust-profile development
colossus plugins enable example-plugin --digest sha256:MANIFEST_DIGEST
```

Replace placeholders with actual values from output. A `required` trust profile rejects
unsigned/unmatched content. `optional` accepts it as untrusted; `disabled` enforces digest
integrity only. Enabling untrusted content requires explicit approval. Do not weaken an
existing profile just to make an example install. Ask the user to select an appropriate
profile when their requested workflow requires one and none exists.

Use standard Cosign tooling and OCI referrers for signatures/attestations; Colossus
does not provide an alternate signing envelope. Registry profiles specify exact registry,
token-service, and permitted redirect origins, CA roots, credential references, and trust.
Docker authentication is opt-in, not an ambient fallback.

```sh
colossus plugins push ./example-plugin.oci registry.example/team/example-plugin:v1 --registry production
colossus plugins pull registry.example/team/example-plugin@sha256:MANIFEST_DIGEST --registry production --output ./import.oci
colossus plugins export example-plugin --output ./example-plugin-layout.tar
colossus plugins install --archive ./example-plugin-layout.tar --trust-profile offline
```

An OCI layout with multiple candidate manifests requires an exact `--digest` on import.
Air-gap exports retain available signatures and attestations. Import and verification use
local trust roots/evidence and do not require registry access. Creating a layout does not
install or enable it; pushing publishes content and needs user authorization.

Background draft: https://github.com/ThomasVitale/agents-skills-oci-artifacts-spec
Colossus's media types and whole-plugin unit are its own OCI profile.
