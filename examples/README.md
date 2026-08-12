# Colossus examples

The example suites are deliberately explicit about prerequisites, authority, and
expected results. Run them from a development workspace after reviewing the files.

| Directory | Purpose |
| --- | --- |
| `asks/` | Numbered prompts for testing common agent behaviors with a configured model |
| `sdk/` | Cross-language durable-run clients and public-API scenarios |
| `workflows/` | Strict durable workflow definitions covering control flow, gates, recovery, and model steps |
| `themes/` | Presentation theme examples |
| `observability/` | Development-only Kubernetes Colossus + Grafana LGTM smoke environment |

Start with `asks/01-model-smoke.txt` for a provider check or
`workflows/01-control-flow-lab.yaml` for a deterministic workflow check. Use
`sdk/scenarios/01-model-smoke.txt` to run the same provider through an enrolled
application SDK.
