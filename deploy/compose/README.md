# Colossus with Docker Compose

Run the web app, documentation and persistent Hosted Chat on one Linux Docker
engine. Use Docker Desktop's Linux engine on Windows or macOS. Setup needs
PowerShell 7.2+; normal operation needs Docker Compose 2.24+ only.

From this directory in a release server bundle, initialize once and start:

```powershell
pwsh ./init.ps1
docker compose up -d --wait
```

In a source checkout, pass `-ReleaseTag` to `init.ps1` with a published release
tag from [Colossus Releases](https://github.com/obscuritylabs/Colossus/releases).
`.env.example` lists the non-secret image, project and port settings.

Open `https://localhost:9443`, trust the public certificate
`.local/material/browser/ca.pem` in your test browser, and sign in using
`.local/admin-login.txt`. Setup never changes your operating system's trust store.
The one-shot bootstrap container creates **Local Colossus** with Chat enabled and
the offline Echo provider. Configure real providers in the application's normal
connection settings when ready. No Desktop/CLI credentials are imported.

Use Compose normally afterward:

```sh
docker compose ps
docker compose logs -f
docker compose down
docker compose up -d --wait
```

`down` retains named volumes. Keep those volumes and the private `.local/` files;
initialization refuses existing output and never replaces keys. `down --volumes`
permanently deletes this project's databases and setup completion flag. Back up
both PostgreSQL databases and retained keys before deliberate image upgrades.
There is no custom upgrade, backup or restore command.

Only loopback ports are published by default. For another hostname or port, pass
`-HostName`, `-Port` and `-ConnectorPort` during initialization; configure DNS and
firewall rules and explicitly change `COLOSSUS_BIND_ADDRESS` in `.env` if needed.
Browser HTTPS and connector mTLS use separate private authorities. Leaves expire
after one year; renewal is a deliberate operator task.

The checkpoint service shares the Docker host's administrator. For independent
checkpoint custody and high availability, see the
[Cloud deployment guides](https://obscuritylabs.github.io/Colossus/cloud/deployment/).
Agents receive no Docker socket or native host execution.

The GitHub **Compose smoke tests** workflow only tests a temporary CI stack; it
does not deploy your installation. Developers can run `node smoke.cjs` with Node
22+ against a fresh Echo demo to verify HTTPS, authentication, streaming and a
retained conversation. It refuses a non-Echo provider and reuses its saved task
on subsequent runs.
