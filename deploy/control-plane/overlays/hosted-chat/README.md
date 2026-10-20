# Hosted Chat compatibility overlay

This path aliases [the shared PostgreSQL overlay](../hosted-chat-sql/README.md).
Both paths now render two rolling replicas with disposable caches. Production
hosted Chat requires shared SQL storage, external keys and the separately
administered anchor authority. The earlier single-replica RWO deployment is retired.

Use the SQL overlay for new deployments. Retain any existing local volume and
matching keys until a supervised migration is verified: rendering this overlay
does not import or delete previous state.

```sh
kubectl kustomize deploy/control-plane/overlays/hosted-chat
```
