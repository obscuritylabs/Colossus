---
title: Provider and model configuration
description: Configure provider connections, model limits and capabilities, and role routing with practical examples.
audience: operator
type: reference
---

# Provider and model configuration

Colossus separates provider connections from model behavior and runtime routing. This
lets several models share one credential and endpoint, or lets different roles use
different providers without duplicating connection settings.

| Layer | Answers | Examples |
| --- | --- | --- |
| Provider profile | Where and how does Colossus connect? | Adapter kind, base URL, credential reference, Chat Completions token parameter, timeout |
| Model profile | Which model is used and what may Colossus send? | Model ID, token limits, reasoning effort, tool calls, streaming |
| Model role | Which model profile handles this job? | Primary agent, summarizer, subagent, research worker |

Use this page to construct the YAML. For credential setup and live diagnostics, see
[Providers and routing](../../admin/providers-routing.md).

For a task-oriented setup path, start with
[Connect a model provider](../../use/providers/index.md). This page remains the
canonical owner for provider and model field semantics, validation rules, and adapter
compatibility boundaries.

## Choose a starting point

| Scenario | Provider kind | Credential | Sandbox destination |
| --- | --- | --- | --- |
| Offline smoke testing | `echo` | None | None |
| Codex/ChatGPT subscription | `open_ai_codex` | `codex:default` | `https://chatgpt.com` and `https://auth.openai.com` |
| OpenAI Responses endpoint | `open_ai_responses` | Usually `env:VARIABLE` | Exact HTTPS origin under isolation |
| OpenAI-compatible Chat Completions endpoint | `open_ai_compatible` | `env:VARIABLE` or `null` | Exact HTTPS or loopback origin under isolation |
| Desktop-managed local model | `open_ai_compatible` | Injected `host:IDENTIFIER` | Exact loopback origin under isolation |
| Several models or providers | One profile per connection | Per provider profile | Every selected provider origin under isolation |

Start with one provider, one model, and only the required `primary` role. Add specialized
roles after the primary route passes both connection and generation diagnostics.

## Complete single-model example

This example connects to one OpenAI-compatible service and routes all model work through
one model profile:

```yaml
providers:
  profiles:
    primary-provider:
      kind: open_ai_compatible
      baseUrl: https://models.example.com/v1
      credentialReference: env:COLOSSUS_MODEL_TOKEN
      chatCompletionsOutputTokenParameter: max_completion_tokens
models:
  profiles:
    primary-model:
      providerProfile: primary-provider
      model: example-model
      contextWindowTokens: 131072
      maxOutputTokens: 16384
      capabilities:
        toolCalls: true
        streaming: true
  roles:
    primary: primary-model
sandbox:
  networkDestinations:
    - https://models.example.com
```

The `baseUrl` includes the provider's API prefix (`/v1`). Under an isolating boundary,
the sandbox destination is only its canonical origin. Acknowledged full access needs no
duplicate destination, and adding one does not narrow ambient authority. Colossus
appends the operation path, such as `/chat/completions`, to the configured base URL.

## Provider profiles

Each entry under `providers.profiles` is a named connection. Profile names must be
nonempty and are referenced by model profiles.

### `kind`

| Value | Transport | Configuration rules |
| --- | --- | --- |
| `echo` | Deterministic, local, network-free response | `baseUrl` and `credentialReference` must both be `null` or omitted |
| `open_ai_codex` | Subscription-backed OpenAI Responses API | Forbids `baseUrl`; requires `credentialReference: codex:default`; uses the fixed ChatGPT Codex backend |
| `open_ai_responses` | OpenAI Responses API | Requires `baseUrl`; Colossus appends `/responses` and `/models` |
| `open_ai_compatible` | OpenAI-compatible Chat Completions API | Requires `baseUrl`; Colossus appends `/chat/completions` and `/models` |

The built-in `echo` route is useful for configuration, storage, policy, and terminal
smoke tests. It does not validate a network provider or real model behavior.

### `baseUrl`

`baseUrl` is the API version prefix, not the complete generation endpoint:

```yaml
providers:
  profiles:
    openai:
      kind: open_ai_responses
      baseUrl: https://api.openai.com/v1
      credentialReference: env:OPENAI_API_KEY
```

The URL must:

- Use HTTP or HTTPS and include a host.
- Under an isolating boundary, use HTTPS unless the host is exact loopback
  (`localhost` or a loopback IP address).
- Under acknowledged full access, a canonical non-loopback plaintext HTTP URL is also
  accepted; it has no TLS confidentiality or server authentication and may expose the
  provider credential and request content in transit.
- Contain no username, password, query, or fragment.
- Include any required API prefix, such as `/v1`.
- Omit `/responses`, `/chat/completions`, and `/models`; Colossus adds those paths.

`open_ai_codex` is the exception: omit `baseUrl`. Colossus pins that adapter to
`https://chatgpt.com/backend-api/codex` so a ChatGPT bearer and account identifier cannot
be redirected to an operator-configured host.

A trailing slash is normalized away. Under an isolating boundary, add only the
canonical origin—scheme, host, and effective port—to
`sandbox.networkDestinations`:

```yaml
sandbox:
  networkDestinations:
    - https://api.openai.com
```

For a private certificate authority, configure
[`network.caBundlePath`](network.md) separately.

### `credentialReference`

Credentials are references, never literal values:

| Form | Use |
| --- | --- |
| `codex:default` | File-backed ChatGPT sign-in created by `colossus codex login`; accepted only by `open_ai_codex` |
| `env:VARIABLE` | Standard CLI, daemon, worker, and unattended deployments |
| `host:IDENTIFIER` | Application-managed runtimes that inject an in-memory credential resolver |
| `null` | Credential-free endpoints, normally local development services |

```yaml
credentialReference: env:COLOSSUS_MODEL_TOKEN
```

The standard CLI and daemon do not interpret `host:` identifiers as secret values. That
form is for an embedding application, such as the desktop-managed local runtime. A
credential is resolved only after policy authorizes the provider effect, and its value
is removed from released results and diagnostics.

`codex:default` reads `$CODEX_HOME/auth.json`, or `~/.codex/auth.json` when that variable
is unset. `CODEX_HOME` must be absolute when set. The file must be a regular non-symlink
file and, on Unix, inaccessible to group and other users. `colossus codex login` and
`colossus codex status` report completion only after this same runtime validation
succeeds; `colossus codex logout` verifies that no usable credential remains and rejects
an unsafe store it cannot verify. Tokens remain late-bound and zeroize when dropped.
Colossus refreshes an expiring access token only through the fixed
`https://auth.openai.com/oauth/token` endpoint and atomically returns the rotated values
to the same file. Grant both
`https://chatgpt.com` and `https://auth.openai.com` in
`sandbox.networkDestinations` under an isolating boundary; acknowledged full access
authorizes both fixed HTTP(S) origins without duplicate grants.
The adapter advertises its separately audited Codex wire-contract version in the
backend's `version` header and model-catalog query; its `User-Agent` continues to identify
the actual Colossus build. A Colossus release must review the matching official Codex
request contract before advancing that compatibility version.
Streaming requests also set `Accept: text/event-stream`; the JSON `stream` flag alone
does not negotiate the subscription backend's SSE response transport.
The fixed Codex backend may omit the response `Content-Type`; only this adapter accepts
an absent header and still requires the body to pass strict SSE and Responses-event
validation. A conflicting response media type remains an error.

Provider credentials are resolved by the in-process provider adapter. They do not need
an entry in `sandbox.environment` unless a separate sandboxed process also needs that
variable.

### `chatCompletionsOutputTokenParameter`

`chatCompletionsOutputTokenParameter` selects how an `open_ai_compatible` profile
projects the model profile's canonical `maxOutputTokens` limit onto Chat Completions
requests:

| Value | Request behavior |
| --- | --- |
| `max_tokens` | Send the limit as `max_tokens`; this legacy-compatible mode is the default when the field is omitted |
| `max_completion_tokens` | Send the limit as `max_completion_tokens` for models that require the modern field |
| `omit` | Send neither output-token parameter; use only when the endpoint rejects both fields or owns the limit itself |

The setting applies equally to streaming and non-streaming requests. Colossus never
sends both fields, does not infer the mode from a model name, and does not retry a
rejected request with another parameter. The `open_ai_responses` adapter continues to
send `max_output_tokens`; the subscription-backed `open_ai_codex` adapter keeps its
separately defined Responses contract. Setting this field on either Responses adapter
or on `echo` is a configuration error.

Keep one canonical token budget under the model profile:

```yaml
providers:
  profiles:
    modern-chat:
      kind: open_ai_compatible
      baseUrl: https://models.example.com/v1
      credentialReference: env:COLOSSUS_MODEL_TOKEN
      chatCompletionsOutputTokenParameter: max_completion_tokens
models:
  profiles:
    primary:
      providerProfile: modern-chat
      model: example-model
      contextWindowTokens: 128000
      maxOutputTokens: 16000
      capabilities:
        toolCalls: true
        streaming: true
```

### `timeoutMs`

`timeoutMs` is an optional positive transport ceiling in milliseconds. When omitted,
Colossus uses `300000` (5 minutes) for remote hosts and `900000` (15 minutes) for exact
loopback hosts: `localhost`, IPv4 loopback, or IPv6 loopback. Private and LAN addresses
that are not loopback use the remote default. An explicit positive value always wins.
The resolved timeout independently bounds model-catalog and generation requests made
through that provider profile.

With the built-in policy, this provider timeout is not silently reduced to
`sandbox.timeoutMs`; the sandbox limit continues to govern ordinary sandbox effects. An
OPA decision may impose a stricter provider obligation. Colossus does not automatically
retry an ambiguous failed generation request.

## Model profiles

Each entry under `models.profiles` selects an exact provider connection and declares the
model metadata Colossus needs to shape requests safely:

| Field | Meaning |
| --- | --- |
| `providerProfile` | Name of an existing entry under `providers.profiles` |
| `model` | Exact nonempty model identifier sent to the provider |
| `contextWindowTokens` | Total model context window; at least `1024` |
| `maxOutputTokens` | Positive output reservation that leaves room for input and safety margin |
| `reasoningEffort` | Optional exact effort: `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, or `ultra` |
| `capabilities.toolCalls` | Whether Colossus may send tool definitions and structured tool history |
| `capabilities.streaming` | Whether Colossus requests the provider's streaming transport |

Colossus does not infer context size or capabilities from a model catalog. Set these
fields from the provider's model documentation. `models doctor` exercises a request
shaped by the configured values, but it cannot prove that a declared context-window
number matches the provider's actual limit.

### Token budget calculation

Colossus reserves output capacity and a safety margin before deciding how much input can
be sent:

```text
safety margin = max(ceil(context window / 10), 512)
input budget  = context window - max output - safety margin
```

For this profile:

```yaml
models:
  profiles:
    general:
      providerProfile: primary-provider
      model: example-general
      contextWindowTokens: 128000
      maxOutputTokens: 16000
      capabilities:
        toolCalls: true
        streaming: true
```

the safety margin is 12,800 tokens and the effective input budget is 99,200 tokens.
Colossus compacts against that input budget using a conservative byte-based estimator.
An individual request may lower `maxOutputTokens`, but it cannot exceed the configured
maximum.

For `open_ai_codex`, this value remains a Colossus context and output reservation; the
subscription-backed Codex request contract does not accept the public Responses API
`max_output_tokens` field, so Colossus omits that field on the wire. Other runtime,
stream, and sandbox output bounds still apply.

Avoid copying a context-window number from a different model variant. Configuration is
rejected if the output and safety reservations consume the whole window.

### Reasoning effort

Set `reasoningEffort` on a model profile when every turn through that profile should use
an explicit reasoning level:

```yaml
models:
  profiles:
    codex:
      providerProfile: codex-provider
      model: YOUR_CODEX_MODEL_ID
      contextWindowTokens: 128000
      maxOutputTokens: 16000
      reasoningEffort: high
      capabilities:
        toolCalls: true
        streaming: true
```

Omit the field to use the provider/model default. Colossus does not infer model support,
downgrade an unsupported level, or retry with another level. The provider will reject an
unsupported combination.

The Responses adapters send `reasoning: { effort: ... }`. The OpenAI-compatible Chat
Completions adapter sends `reasoning_effort`. The accepted configuration vocabulary is
the union needed by those adapters; `ultra` is available in current Codex model catalogs
but is not a portable level across providers.

### Capabilities

Set `toolCalls: true` only when the selected endpoint and model support function tools.
When it is `false`, Colossus omits tool definitions and rejects structured tool history
for that route. This is appropriate for a text-only summarizer or a local model without
reliable tool support.

Set `streaming: true` when the provider supports the adapter's streaming response
contract. Set it to `false` for a compatible server that implements only complete JSON
responses. Capability flags shape requests; they do not grant access to tools or
actions.

## Role routing

`models.roles` maps a fixed logical role to a model profile. `primary` is required. An
unconfigured specialized role falls back to the `primary` model profile.

| Role | Work routed through it |
| --- | --- |
| `primary` | Ordinary agent turns; required fallback for every unmapped specialized role |
| `risk_evaluator` | Low-risk automatic approval assessment |
| `context_summarizer` | Model-assisted context compaction |
| `subagent_default` | Child-agent model calls |
| `research_planner` | Research planning |
| `research_worker` | Parallel source investigation |
| `research_synthesizer` | Final research synthesis |

Unknown role names and role targets that do not name an existing model profile are
rejected.

### Multi-model routing example

Two model profiles can share one provider connection. This example routes ordinary and
final synthesis work to a larger model while using a smaller text-only model for
summarization:

```yaml
providers:
  profiles:
    hosted:
      kind: open_ai_compatible
      baseUrl: https://models.example.com/v1
      credentialReference: env:COLOSSUS_MODEL_TOKEN
models:
  profiles:
    general:
      providerProfile: hosted
      model: example-general
      contextWindowTokens: 128000
      maxOutputTokens: 16000
      capabilities:
        toolCalls: true
        streaming: true
    summarizer:
      providerProfile: hosted
      model: example-small
      contextWindowTokens: 32000
      maxOutputTokens: 4000
      capabilities:
        toolCalls: false
        streaming: true
  roles:
    primary: general
    context_summarizer: summarizer
    research_synthesizer: general
```

Roles omitted from this example—including `risk_evaluator` and `subagent_default`—fall
back to `general` through the `primary` mapping.

## Advanced examples

### OpenAI Responses

```yaml
providers:
  profiles:
    openai:
      kind: open_ai_responses
      baseUrl: https://api.openai.com/v1
      credentialReference: env:OPENAI_API_KEY
models:
  profiles:
    openai-primary:
      providerProfile: openai
      model: YOUR_MODEL_ID
      contextWindowTokens: 128000
      maxOutputTokens: 16000
      capabilities:
        toolCalls: true
        streaming: true
  roles:
    primary: openai-primary
sandbox:
  networkDestinations:
    - https://api.openai.com
```

Replace the model ID and limits with the exact values for the selected model.

### Local OpenAI-compatible server

Exact loopback endpoints may use HTTP and omit credentials:

```yaml
providers:
  profiles:
    local:
      kind: open_ai_compatible
      baseUrl: http://127.0.0.1:11434/v1
      credentialReference: null
models:
  profiles:
    local-primary:
      providerProfile: local
      model: local-model
      contextWindowTokens: 32768
      maxOutputTokens: 4096
      capabilities:
        toolCalls: true
        streaming: true
  roles:
    primary: local-primary
sandbox:
  networkDestinations:
    - http://127.0.0.1:11434
```

Set `toolCalls` or `streaming` to `false` if the local server or selected model does not
implement that contract. A server that is still loading may return HTTP 503; Colossus
reports a recoverable temporary-unavailability error but does not retry the turn
implicitly.

### Offline echo route

```yaml
providers:
  profiles:
    echo:
      kind: echo
      baseUrl: null
      credentialReference: null
models:
  profiles:
    echo:
      providerProfile: echo
      model: echo
      contextWindowTokens: 32768
      maxOutputTokens: 4096
      capabilities:
        toolCalls: true
        streaming: true
  roles:
    primary: echo
```

### Application-managed credential

An embedding application can keep the provider secret outside both YAML and the process
environment:

```yaml
providers:
  profiles:
    managed-local:
      kind: open_ai_compatible
      baseUrl: http://127.0.0.1:1234/v1
      credentialReference: host:managed-local-primary
```

This configuration requires a host credential resolver supplied by the application. It
will not authenticate through the standard CLI or daemon composition.

## Tool compatibility at the provider boundary

Canonical Colossus tool names remain dotted in access configuration, policy, audit, and
dispatch. For network provider requests, each `.` is projected to `_`; for example,
`filesystem.write` becomes `filesystem_write`. Returned aliases are restored before
runtime handling. Names that cannot fit the portable 64-byte `[A-Za-z0-9_-]` contract,
or names that collide after projection, fail locally before a request is sent.

Every canonical tool schema must declare an object at its root. Colossus creates a
provider-facing copy that removes root-level `oneOf`, `anyOf`, `allOf`, `enum`, and
`const`. Responses requests use non-strict function tools. Chat Completions requests
omit `strict` and also remove recursive `maxLength` annotations for compatible servers
that compile tool schemas into bounded grammars. The original schema remains unchanged
and is validated in full before policy or dispatch; provider compatibility never widens
tool authority.

## Common configuration mistakes

| Symptom | Check |
| --- | --- |
| A role target is rejected | Roles point to model profile names, not provider profile names or raw model IDs |
| Generation uses the wrong endpoint | Configure the API prefix in `baseUrl`; omit `/responses` and `/chat/completions` |
| A remote URL is rejected | Use HTTPS and remove URL credentials, query parameters, and fragments |
| The provider origin is denied under isolation | Add only the canonical origin to `sandbox.networkDestinations` |
| A credential is unavailable | Use `env:VARIABLE` and inject its value into the Colossus process; do not put the value in YAML |
| A Codex sign-in is unavailable | Run `colossus codex status`, then `colossus codex login`; ensure the file-backed auth file is owner-only |
| A `host:` credential is unavailable | Run through an application that supplies the matching in-memory resolver |
| The context profile is rejected | Correct the model window or reduce `maxOutputTokens` so the input budget remains positive |
| A compatible server returns HTTP 400 | Verify model ID, tool support, streaming support, and the server's OpenAI compatibility |
| A compatible server returns HTTP 503 | Wait for the model to load, rerun diagnostics, and explicitly resubmit the turn |

## Validate the result

Inspect routing without making a generation request:

```bash
colossus --config .colossus/config.yaml provider profiles
colossus --config .colossus/config.yaml models profiles
colossus --config .colossus/config.yaml models routes
colossus --config .colossus/config.yaml models route primary
```

Then test the provider connection and exact model separately:

```bash
colossus --config .colossus/config.yaml provider doctor PROFILE
colossus --config .colossus/config.yaml provider models PROFILE
colossus --config .colossus/config.yaml models doctor MODEL_PROFILE
```

`provider doctor` checks the connection and model-catalog boundary. `models doctor`
makes one bounded generation probe using the selected model profile, including a
representative tool schema when `toolCalls` is enabled. Probe response content and
credential values are not printed.

Return to the [configuration overview](../configuration.md).
