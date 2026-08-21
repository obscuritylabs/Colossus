# SearXNG development service

Start a loopback-only SearXNG instance for local search examples:

```bash
docker compose -f examples/services/searxng/compose.yml up -d --wait
```

The default secret and unpinned image are for local development only. Configure a
reviewed secret and image digest before using this stack in a shared environment. Point
a top-level Colossus `search` profile at `http://127.0.0.1:8888/search` and authorize
the exact `http://127.0.0.1:8888` network destination when using declared authority.

Stop the service with:

```bash
docker compose -f examples/services/searxng/compose.yml down
```
