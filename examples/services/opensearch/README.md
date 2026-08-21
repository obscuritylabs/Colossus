# OpenSearch development service

Start the loopback-only OpenSearch stack for integration development:

```bash
docker compose -f examples/services/opensearch/compose.yml up -d --wait
```

This stack disables the OpenSearch security plugin and uses an unpinned image by
default. Do not expose it beyond the local host or treat it as a production deployment.

Stop the service with:

```bash
docker compose -f examples/services/opensearch/compose.yml down
```
