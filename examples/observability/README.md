# Development OpenTelemetry LGTM example

This example runs a locally built `colossus:dev` image beside
`grafana/otel-lgtm:0.29.0` in one Kubernetes pod. Colossus exports traces, metrics, and
logs to the sidecar's loopback OTLP receiver. The deployment uses disposable storage,
the echo provider, plaintext loopback OTLP, default Grafana credentials, and no durable
collector data.

**This example is for development and smoke testing only. It is unsuitable for
production.**

Build the Colossus image into the cluster's local image store, then run:

```bash
kubectl apply -f examples/observability/lgtm-kubernetes.yaml
kubectl -n colossus-observability-dev rollout status deployment/colossus-lgtm
kubectl -n colossus-observability-dev port-forward service/colossus-lgtm 3000:3000
```

Open `http://127.0.0.1:3000` and sign in as `admin` / `admin`. Create a run through the
worker's public API from a test client in the pod, or execute a worker-routed local run,
then inspect Tempo, Mimir/Prometheus, and Loki. Expected signals are an RPC/agent/model
trace (and a tool span when the prompt uses a tool), `gen_ai.*` metrics without run or
user IDs, and correlated `colossus.journal.appended` log records.

Delete all disposable resources with:

```bash
kubectl delete namespace colossus-observability-dev
```
