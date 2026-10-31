---
title: Deploy Colossus Cloud
description: Choose the installation and operating guides for your own Colossus Control Plane.
audience: operator
type: concept
icon: lucide/server
---

# Deploy Colossus Cloud

Choose Docker Compose for a persistent single-host installation with Chat enabled
by default, or Helm for Kubernetes 1.34+. Both reuse the released Control Plane,
browser app and documentation images. Helm supports GKE, EKS, OpenShift and
disconnected registries.

<div class="grid cards" markdown>

-   :lucide-server:{ .lg .middle } **Install on one Docker host**

    ---

    Start the web application, databases, documentation and Hosted Chat with Compose.

    [Install with Compose :lucide-arrow-right:](compose.md)

-   :lucide-package:{ .lg .middle } **Install**

    ---

    Prepare the cluster, set hostnames, and install a pinned release.

    [Install with Helm :lucide-arrow-right:](kubernetes.md)

-   :lucide-settings:{ .lg .middle } **Configure**

    ---

    Choose routing, external database TLS, sign-in, and platform settings.

    [Configure your installation :lucide-arrow-right:](configuration.md)

-   :lucide-messages-square:{ .lg .middle } **Enable hosted chat**

    ---

    Add browser-based execution with external keys and an independent anchor service.

    [Set up hosted chat :lucide-arrow-right:](hosted-chat.md)

-   :lucide-database-backup:{ .lg .middle } **Operate**

    ---

    Back up state and identities, upgrade, and recover from interruptions.

    [Upgrade and recover :lucide-arrow-right:](operations.md)

</div>

## What the default Helm installation provides

| Component | Default |
| --- | --- |
| Control Plane | Three replicas; CPU autoscaling from three to six |
| Documentation | Three replicas at the browser origin's `/docs/` path |
| PostgreSQL | One persistent instance with a 20 GiB volume |
| Sign-in | Local accounts with a generated initial administrator password |
| Placement | Hard spreading across three eligible zones |
| Hosted chat | Disabled until explicitly configured |

The bundled database is a single point of failure. Choose external managed
PostgreSQL when database availability or encrypted database transport is required.
Gateway controllers, storage, DNS, browser certificates, and enforced network policy
remain cluster prerequisites.

For every supported value and Secret key, use the
[chart reference](https://github.com/obscuritylabs/Colossus/tree/main/deploy/control-plane/chart).
