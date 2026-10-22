---
title: "ADR 0010: Control Plane Helm and database trust boundary"
description: Package the Control Plane with explicit bundled-database trust while preserving external TLS and runtime authority.
audience: developer
type: concept
---

# ADR 0010: Control Plane Helm and database trust boundary

- Status: Accepted
- Date: 2026-10-21

## Context

Control Plane installations need one release-matched package for Kubernetes, including
OpenShift and disconnected environments. The former Kustomize examples assumed an
external PostgreSQL database and required verified database TLS. The deployment contract
now includes an optional bundled PostgreSQL dependency in the release namespace, with
plaintext transport inside an operator-selected cluster trust boundary. That exception
must be explicit rather than weakening the existing external-database or fixture rules.

## Decision

Helm replaces the production Kustomize examples. The package includes the server, public
documentation and a pinned, vendored PostgreSQL dependency. Chart and application
versions advance together; an independently validated tag candidate supplies the exact
OCI chart bytes published to Docker Hub. Platform operators retain ownership of storage,
ingress controllers, gateways, meshes, DNS and trust material.

The cloud PostgreSQL adapter exposes an explicit `in_cluster` transport configuration,
bound to one configured Service and namespace. Its connection parser accepts only that
exact Service DNS hostname, rejects multiple hosts and host-address overrides, and
selects plaintext deliberately. This identifies the requested trust boundary; it does
not prove that DNS, Services, pods, namespace membership or CNI policy are trustworthy.
The Helm default supplies a private database Service and a NetworkPolicy, with CNI
enforcement remaining a platform prerequisite.

External database deployments retain verified public-root or private-CA TLS. The
existing disabled-TLS mode remains restricted to loopback/Unix fixtures and is rejected
by the production server. Runtime journal database configuration is independent and
receives no corresponding exception.

Shared hosted chat stores runtime journals through the separate journal adapter.
It therefore requires external verified-TLS PostgreSQL; both server validation and
the Helm chart reject shared hosted execution with the bundled plaintext database.

Browser TLS termination and connector TLS passthrough remain separate. The connector
CA and enrollment signing key are application-held identities; chart configuration
does not grant cloud roles any additional native runtime execution authority.

## Consequences

Cluster operators explicitly accept confidentiality and integrity risks for bundled
plaintext database traffic and must enforce the selected network boundary. Deployments
requiring database transport encryption use the external database option. Three
Control Plane replicas provide application redundancy; one persistent PostgreSQL
instance does not provide database HA.

Generated credentials and the connector CA outlive ordinary Helm uninstall alongside
retained database storage. Offline/GitOps rendering uses operator-managed existing
Secrets to avoid replacing durable identities with newly generated values. Database
migrations remain application startup operations; Helm rollback does not reverse them.
Static rendering and configuration checks do not establish cluster installation or
upgrade acceptance.

Current values, Secret contracts and recovery procedures belong to the
[operator guide](../../cloud/deployment/index.md) and
[chart source](https://github.com/obscuritylabs/Colossus/tree/main/deploy/control-plane/chart).
