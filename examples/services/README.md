# Optional development services

These Compose stacks support local examples and integration development. They are not
part of the Colossus runtime, release image, or production deployment contract.

| Directory | Purpose | Security note |
| --- | --- | --- |
| `searxng/` | Local web-search backend for agent and research routes | Loopback-only port; replace the development secret before sharing the service |

Run commands from the repository root so relative paths remain predictable. Pin image
versions in any reproducible or shared environment.
