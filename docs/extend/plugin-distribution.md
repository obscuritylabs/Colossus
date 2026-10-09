---
title: Distribute and trust plugins
description: Publish signed OCI plugins, configure registry trust, and move packages into offline environments.
audience: operator
type: how-to
---

# Distribute and trust plugins

Use this guide when you publish a plugin, configure a private registry, or import a signed package offline. For everyday use, start with [Agent Plugins](plugins.md).

## Add a published plugin

First [configure a registry profile and signing policy](../reference/configuration/extensions.md).
Replace the reference with a published version from your registry:

```bash
colossus plugins add oci://registry.example/acme/review:v1 --registry production
```

The runtime resolves the tag once, verifies its signature under the matching registry profile, installs it, and activates the exact returned digest. If verification or activation fails, the previously active version remains selected. MCP connections still need explicit workspace configuration.

## Registries and trust

Registry profiles declare an exact origin, allowed token-service and blob-redirect
origins, per-origin CA roots, authentication, and a trust profile. No registry is contacted
at startup and no ambient Docker credentials are used unless `auth.kind: docker` is
selected explicitly. Bearer/basic values remain credential references. Docker helpers
require an exact configured executable and run through the normal process permit and audit
boundary.
The built-in `obscuritylabs` profile allows anonymous pulls from `https://ghcr.io` and
requires a keyless Sigstore certificate issued by GitHub Actions for the exact
`obscuritylabs/colossus-plugins` main-branch plugin workflow. It does not trust every
artifact on GHCR. Explicit workspace configuration can replace or remove this profile.
Docker configuration is opened only inside an authorized registry transfer, and its file
must be covered by that transfer's permit. Denied transfers do not inspect credentials.

Trust profiles are `required` by default. `optional` admits unmatched content as untrusted;
enabling it requires explicit approval. `disabled` deliberately applies digest integrity
only and has the same explicit untrusted-enable requirement. Signature verification is
in-process Sigstore/Cosign using configured public keys or keyless issuer/subject identity,
with optional local trust roots and bundled transparency evidence for disconnected use.
Verification and installation require read grants for the selected profile's public-key
and trust-root files. The built-in policy requests approval for paths outside existing
workspace grants; an external policy must supply those grants explicitly. Re-verifying
an installed plugin uses the same checks.

```bash
colossus plugins pull registry.example/acme/review:v1 \
  --registry production --output ./review.oci
colossus plugins push ./review.oci registry.example/acme/review:v1 \
  --registry production
colossus plugins install --reference registry.example/acme/review@sha256:DIGEST \
  --registry production
```

Cosign signatures and attestations remain standard OCI referrers. Pulls also read the
OCI referrers tag fallback when a registry does not serve the referrers API, verifying
each attached manifest's digest and exact subject before checking its signature.
Colossus does not invent a signing envelope.

## Import an offline package

```bash
colossus plugins verify ./review.oci --trust-profile default
colossus plugins install --layout ./review.oci --trust-profile default
colossus plugins show example-plugin
colossus plugins enable example-plugin --digest sha256:MANIFEST_DIGEST
```

Replace the name and digest with the installation receipt. The advanced `install` command deliberately leaves the candidate disabled; `add` combines verification, installation, and trusted activation. Layouts containing several candidates require `--digest` on import. Unsigned OCI content admitted by an optional or disabled trust profile still needs separate approval through `enable --allow-untrusted`; `add` does not bypass that approval.

## Lifecycle and air gaps

```bash
colossus plugins list
colossus plugins show example-plugin
colossus plugins disable example-plugin
colossus plugins uninstall example-plugin --digest sha256:DIGEST
colossus plugins gc
colossus plugins export example-plugin --output ./example-plugin-layout.tar
```

The dedicated `plugins/state.redb` journal serializes lifecycle writers. Running snapshots
lease their immutable content, so disable, uninstall, or garbage collection affects only
later runs. Export carries the plugin manifest, blobs, signatures, and attestations and
does not open a network connection during import.

## Bundled plugin versions

Colossus includes its own `colossus` plugin with `coding`, `offline-dev`,
`security-review`, `plugin-authoring`, and `schedule-task` skills. An explicit
Colossus home receives the bundled version on startup, without a registry pull.
SDK runtimes without an explicit home remain isolated.

The **Bundled with Colossus** label records executable ownership. Signature
verification remains a separate trust claim. Imported and local sources cannot
use the reserved `colossus` name.

The executable manages the bundled version, including upgrades and rollbacks.
A user's disabled preference survives version changes. Existing runs retain their
original snapshots. You can inspect, verify, export, enable, or disable the bundled
plugin; independent update and uninstall are unavailable.
