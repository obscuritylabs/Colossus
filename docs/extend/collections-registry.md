---
title: Collections and registry
description: Build, sign, verify, install, pull, and push deterministic collections of Colossus packs and skills.
audience: developer
type: how-to
---

# Collections and registry

## Goal

Build a reproducible signed collection from reviewed packs and skills, verify it locally,
then transport the same artifact through a create-only registry.

## Prerequisites

- A staged directory containing immediate `packs/NAME` and `skills/NAME` entries.
- Every pack independently signed and trusted.
- An Ed25519 collection signing seed available only through an environment reference.
- An explicit UTC creation timestamp.
- For registry transport, an HTTPS endpoint and optional bearer credential reference.

## Steps

### 1. Derive the public signing identity

Keep the private seed late-bound: make `COLLECTION_SEED` available to the Colossus
process, but do not place its value in configuration, a manifest, or a command argument.
Derive its safe public identity through the credential reference:

```bash
colossus --config .colossus/config.yaml --output json \
  bundle key-info \
  --signing-key-reference env:COLLECTION_SEED
```

Record the returned `public_key` and `key_id`. The command returns only public material;
the signing seed remains behind `env:COLLECTION_SEED`.

### 2. Bind publisher trust

Replace `BASE64_PUBLIC_KEY_FROM_KEY_INFO` with the `public_key` returned in the previous
step:

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  packs trust add example \
  --public-key "BASE64_PUBLIC_KEY_FROM_KEY_INFO"
```

Confirm the trust result reports the same `key_id` as `bundle key-info`. Collection
building performs strict verification before publishing the destination, so this exact
publisher/key binding must exist before the build.

### 3. Build the deterministic collection

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  collections build ./staged ./collection \
  --name starter --version 1.0.0 --publisher example \
  --created-at 2026-07-16T12:00:00Z \
  --signing-key-reference env:COLLECTION_SEED
```

For identical staged bytes, timestamp, metadata, and signing seed, the build is
reproducible. Exact pack dependency closure must be present and acyclic. The build
resolves the private seed only from `env:COLLECTION_SEED`, signs the manifest, verifies
the signed result against the established `example` trust binding, and then publishes
the destination.

### 4. Verify the whole collection

```bash
colossus --config .colossus/config.yaml collections verify ./collection
```

Verification checks the collection inventory, signature, every nested artifact, pack
publisher trust, and skill data-only boundaries.

### 5. Test installation in a clean destination

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  collections install ./collection
```

Installation stages the complete set, refuses existing destinations, commits pack
lifecycle events as one journal batch, and rolls back synchronous failure.

### 6. Push with create-only semantics

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  registry push ./collection \
  https://registry.example/v1/starter/1.0.0 \
  --credential-reference env:REGISTRY_TOKEN
```

The origin and credential variable must be explicitly granted. A conflict counts as
replay success only when the server returns the same content hash.

### 7. Pull and verify into a clean path

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  registry pull https://registry.example/v1/starter/1.0.0 \
  ./starter \
  --credential-reference env:REGISTRY_TOKEN
colossus --config .colossus/config.yaml collections verify ./starter
```

Registry transport uses the same signed collection as a deterministic archive. No
registry is contacted during local build, verification, or installation.

## Expected result

The locally verified collection has one stable identity. Push never overwrites different
content, and pull publishes only a completely downloaded and verified artifact.

## Verification

Build the same staged input twice with identical parameters and compare collection
identity. Verify the pulled artifact again before installation and compare it with the
published content hash.

## Failure path

- **Build is not reproducible:** compare staged bytes, explicit timestamp, metadata,
  dependency closure, and signing seed identity.
- **Collection signature is untrusted:** rerun `bundle key-info` for the exact signing
  reference and confirm `packs trust add` bound that public key to the collection
  publisher before rebuilding.
- **Nested pack is untrusted:** establish its exact publisher/key binding before
  rebuilding.
- **Destination exists:** choose a clean path; install and pull do not merge.
- **Registry conflict has a different hash:** treat it as an immutable-name collision
  and publish under a new intended identity.
- **Transport outcome is unknown:** inspect registry state before repeating a push.

## Next step

Consult [Extension manifests](../reference/extension-formats.md) for exact collection,
pack, and skill formats, and [Offline operation](../admin/offline-airgap.md) for isolated
distribution.
