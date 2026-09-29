# Colossus examples

The example suites are deliberately explicit about prerequisites, authority, and
expected results. Run them from a development workspace after reviewing the files.

| Directory | Purpose |
| --- | --- |
| [`desktop-setup/`](desktop-setup/README.md) | Sample company setup folder, packaging commands, and a Desktop manual test checklist |
| `asks/` | Numbered prompts for testing common agent behaviors with a configured model |
| `sdk/` | Cross-language durable-run clients and public-API scenarios |
| `workflows/` | Strict durable workflow definitions covering control flow, gates, recovery, and model steps |
| `themes/` | Presentation theme examples |
| `observability/` | Development-only Kubernetes Colossus + Grafana LGTM smoke environment |
| `services/` | Optional local service stacks used by examples and integration development |

Start with `asks/01-model-smoke.txt` for a provider check or
`workflows/01-control-flow-lab.yaml` for a deterministic workflow check. Use
`sdk/scenarios/01-model-smoke.txt` to run the same provider through an enrolled
application SDK.

For a hands-on Desktop setup check, start with
[`desktop-setup/README.md`](desktop-setup/README.md). It packages five sample providers
and six models into one importable file and explains what to expect at each step.

Service stacks are development conveniences, not supported production deployments.
Each service directory documents its security posture and startup command.
