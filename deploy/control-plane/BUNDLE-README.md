# Colossus Control Plane server bundle

This archive contains matching Linux server binaries, browser assets, and deployment
files from one Colossus release. Verify the archive against its adjacent `.sha256`
file before extracting it.

| Path | Contents |
| --- | --- |
| `bin/colossus-control-plane` | Control Plane server |
| `bin/colossus-chat-anchor` | Independently administered Hosted checkpoint anchor |
| `web/` | Matching browser application assets |
| `deploy/chart/` | Helm chart, vendored dependency, and deployment examples |
| `deploy/compose/` | Single-host Docker Compose setup and lifecycle helper; Chat enabled by default |
| `LICENSE` | Release license |

Check the binaries without starting a deployment:

```sh
./bin/colossus-control-plane --version
./bin/colossus-control-plane --help
./bin/colossus-chat-anchor --version
./bin/colossus-chat-anchor --help
```

The server and anchor each require their own operator-reviewed configuration. The
anchor must use independent credentials and keys. The bundle contains no user
execution environment or deployment credentials.

The [Cloud deployment guide](https://obscuritylabs.github.io/Colossus/cloud/deployment/)
owns installation, authentication, TLS, configuration, and operating procedures.
Review [Hosted chat setup](https://obscuritylabs.github.io/Colossus/cloud/deployment/hosted-chat/)
before enabling hosted execution, and follow the
[upgrade and recovery guide](https://obscuritylabs.github.io/Colossus/cloud/deployment/operations/)
when replacing an existing deployment.

For Kubernetes, start with the packaged [chart reference](deploy/chart/README.md).
For a single Linux Docker host, start with [Compose](deploy/compose/README.md):

```powershell
pwsh ./deploy/compose/compose.ps1 -Action up -Name local
```

Setup generates private credentials; it does not import native Desktop or CLI state.
The separate release asset ending in `.docker.tar.gz` is the matching offline
container image; verify its own checksum before loading it. Published image tags and
digests are documented in
[release image operations](https://obscuritylabs.github.io/Colossus/develop/releasing/#control-plane-containers-and-vs-code-packages).
