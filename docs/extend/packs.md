---
title: Packs
description: Verify, trust, install, and lifecycle-manage executable Colossus capability packages.
audience: operator
type: how-to
---

# Packs

## Goal

Verify a local capability pack, bind its publisher to an exact Ed25519 key, install it,
and expose only its reviewed tools.

## Prerequisites

- A local pack directory or supported local OCI layout.
- A `colossus.pack.json` whose files, sizes, hashes, capabilities, and permissions match
  the payload.
- The publisher's verified Ed25519 public key.
- Approval for trust and lifecycle mutations.

## Steps

### 1. Verify before changing trust

```bash
colossus --config .colossus/config.yaml packs verify ./pack
```

Verification rejects unlisted payloads, mismatched hashes or sizes, unsafe paths,
symlinks, special entries, invalid signatures, undeclared executable behavior, and
permission inconsistencies.

### 2. Bind publisher and exact key

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  packs trust add PUBLISHER --public-key BASE64_ED25519_PUBLIC_KEY
colossus --config .colossus/config.yaml packs trust list
```

A publisher name alone is not trusted; the binding uses the exact key identity.

### 3. Install and inspect

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  packs install ./pack
colossus --config .colossus/config.yaml packs show PACK_NAME
colossus --config .colossus/config.yaml packs list
```

Unsigned or untrusted external packs are blocked by default. The
`--allow-untrusted` development override is separately approval-gated and never creates
a trust binding.

### 4. Enable and call deliberately

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  packs enable PACK_NAME
colossus --config .colossus/config.yaml config effective
colossus --config .colossus/config.yaml --approval-mode ask \
  packs call PACK_TOOL_NAME
```

Enabled fixed-argument tools become access candidates on the next runtime start.
Their declared permissions are a ceiling, not authority; policy and sandbox still
constrain every call.

## Expected result

The installed lifecycle identifies the exact manifest and publisher key. Only enabled,
reverified, trusted tools with satisfied prerequisites enter the active catalog.

## Verification

Restart Colossus, then compare `packs show`, `config effective`, and `tools list`.
Confirm that executable paths and requested permissions match the reviewed manifest.

## Failure path

- **Signature fails:** stop; do not use the untrusted override to bypass a present
  invalid signature.
- **Payload is undeclared:** update the manifest and signature from reviewed source.
- **Permission ceiling is too broad:** narrow pack and tool declarations before
  installation.
- **Tool remains hidden:** inspect lifecycle, trust, access profile, and exact
  prerequisites.
- **Call outcome is unknown:** reconcile the external effect before retrying.

## Next step

Use the [extension manifest reference](../reference/extension-formats.md) when authoring
a pack, or distribute packs and skills with
[Collections and registry](collections-registry.md).
