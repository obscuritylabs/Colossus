---
title: Administer and secure Colossus
description: Operate Colossus with explicit access, isolation, storage, audit, and recovery controls.
audience: operator
type: concept
---

# Administer and secure Colossus

Colossus treats model calls, filesystem access, processes, network traffic, durable
state, and extensions as separate operational concerns. A provider being connected does
not grant it access to a repository, and an approval does not widen the sandbox.

Start with the operational layer that matches your job:

- [Colossus home and workspace resolution](../reference/colossus-home.md) for
  per-user state, configuration selection, and repository instructions.
- [Configuration recipes](configuration.md) for a safe, inspectable baseline.
- [Providers and routing](providers-routing.md) for hosted or local models.
- [Access and approvals](access-and-approvals.md) for tool visibility and effect decisions.
- [Policy and OPA](policy-opa.md) for local or remote authorization.
- [Sandbox](sandbox.md) for filesystem, process, and network containment.
- [Storage and worker](storage-worker.md) for the canonical journal and multi-client use.
- [Audit, telemetry, and recovery](audit-telemetry-recovery.md) for evidence and incidents.
- [Offline operation](offline-airgap.md) for isolated environments.
- [Troubleshooting](troubleshooting.md) for a symptom-first diagnostic path.

The [Reference](../reference/index.md) section owns exact fields, action names, schemas,
limits, and command surfaces. These operator pages explain how to combine those
contracts safely.
