# Colossus Compose deployment

Run the release-matched web application, documentation, PostgreSQL and Hosted Chat
on one Linux Docker engine. Docker Desktop's Linux engine works on Windows and macOS.
PowerShell 7.2+ prepares the installation; Node 22+ is needed only for `test`.

From a source checkout, select an already published image release explicitly:

```powershell
pwsh ./deploy/compose/compose.ps1 -Action up -Name demo -ReleaseTag v0.11.11-preview.5
pwsh ./deploy/compose/compose.ps1 -Action test -Name demo
```

The release server bundle includes this directory at `deploy/compose/`; its default
image tag comes from the accompanying release chart. The helper resolves tags to
registry digests before starting containers. It never pulls `latest`.

Chat is enabled by default with an offline Echo provider and a ready **Local
Colossus** project. Select real provider connections in the application deliberately.
Use `-CoreOnly` on first installation for a Fleet-only Control Plane.

The default browser address is `https://localhost:9443`; connectors use their own
end-to-end mTLS listener on port 9444. Only loopback is published. Setup prints the
private `admin-login.txt` and public `browser-ca.pem` paths, never the password.
Review the CA before trusting it in your browser; setup does not modify trust stores.

The private default allocation is `~/.colossus-deployments/compose-NAME`. Keep its
files and the Docker volumes. Missing keys or retained volumes are errors, not a
request to generate replacement state. `-StateDirectory` selects another absolute,
owner-private parent. Hostnames, ports and the chat profile are fixed at allocation.

```powershell
pwsh ./deploy/compose/compose.ps1 -Action status -Name demo
pwsh ./deploy/compose/compose.ps1 -Action down -Name demo
pwsh ./deploy/compose/compose.ps1 -Action up -Name demo
```

`down` retains databases, certificates and keys. `down -RemoveData -WhatIf` previews
irreversible deletion of only this installation's owned Docker volumes; omit
`-WhatIf` to perform it. The private allocation stays available for inspection.
After data removal, use a fresh name or restore a backup.

See the [operator guide](https://obscuritylabs.github.io/Colossus/cloud/deployment/compose/) for backup, restore,
upgrade, remote bindings and the single-host trust model. The Hosted anchor has its
own database, secrets and network, but shares the Docker host and its administrator.
This is not an independently administered production checkpoint authority. Use the
Helm deployment with an external authority for that boundary and high availability.
