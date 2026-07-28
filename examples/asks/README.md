# Agent ask examples

These numbered prompts exercise common ways people ask Colossus to work. Unlike a
workflow definition, an ask is ordinary model input: it is intentionally not a new
configuration or execution format.

Review every prompt before running it. Colossus still applies the configured access,
policy, approval, sandbox, tool, and provider boundaries. A prompt can request an
effect, but it cannot grant that effect authority.

## Coverage

| File | Mode | Expected result | Authority exercised |
| --- | --- | --- | --- |
| `01-model-smoke.txt` | `run` | returns exactly `ASK_SUITE_OK` | provider only |
| `02-repository-map.txt` | `run` | concise architecture map with relative file evidence | workspace reads |
| `03-focused-code-review.txt` | `run` | prioritized findings, or an explicit no-findings result | Git and workspace reads |
| `04-failing-test-diagnosis.txt` | `run` in the fixture copy | identifies the defect without editing | test execution and reads |
| `05-plan-a-fix.txt` | `run --plan` in the fixture copy | creates one durable draft plan | bounded reads and Colossus plan state |
| `06-implement-and-verify.txt` | `run` in the fixture copy | fixes the defect and passes tests | execution and workspace mutation |
| `07-subagent-review.txt` | `run` | two child reviews plus a primary synthesis | delegation and workspace reads |
| `08-durable-project-state.txt` | `run` in the fixture copy | creates and lists a task, decision, and memory | Colossus durable state |
| `09-active-skill-review.txt` | `run --skill` | follows one explicitly activated skill | configured skill and required tools |
| `10-source-backed-research.txt` | `research run` | cited claims and a source list | repository and configured search routes |

The prompts use workspace-relative paths. The two review examples are suitable for this
repository. Examples 04–06 and 08 target the small fixture under `fixture/`.

## Run an ask on macOS or Linux

Build Colossus and keep the executable and configuration paths anchored at the
repository root:

```bash
COLOSSUS_BIN="$PWD/target/debug/colossus"
COLOSSUS_CONFIG="$PWD/.colossus/config.yaml"
ASK_TEXT="$(<examples/asks/01-model-smoke.txt)"
"$COLOSSUS_BIN" --config "$COLOSSUS_CONFIG" run "$ASK_TEXT"
```

For streamed progress, add `--stream` after `run`. Released progress goes to stderr and
the final response remains on stdout.

## Run an ask on Windows PowerShell

```powershell
$ColossusBin = Join-Path $PWD "target\debug\colossus.exe"
$ColossusConfig = Join-Path $PWD ".colossus\config.yaml"
$AskText = Get-Content -Raw "examples\asks\01-model-smoke.txt"
& $ColossusBin --config $ColossusConfig run $AskText
```

## Use the disposable implementation fixture

Do not run a mutating example against the checked-in fixture. Copy it first so every
test starts from the same known failing state:

```bash
ASK_WORKSPACE="$(mktemp -d)"
cp -R examples/asks/fixture/. "$ASK_WORKSPACE/"
ASK_TEXT="$(<examples/asks/04-failing-test-diagnosis.txt)"
"$COLOSSUS_BIN" -w "$ASK_WORKSPACE" --config "$COLOSSUS_CONFIG" \
  --approval-mode risk-auto run --max-turns 12 "$ASK_TEXT"
```

The fixture contains one failing Rust test. Ask 04 must diagnose it without changing
the source. Ask 06 should change only the copied workspace and finish with both tests
passing. Use `--approval-mode ask` in an interactive terminal when you want to inspect
each execution and mutation approval. `risk-auto` may approve an eligible low-risk test
command, but never a workspace write.

Smaller local models may repeat a diagnostic command before producing their final
answer. The explicit turn bound keeps that behavior finite; verbose event counts can
still be much larger than the number of visible assistant messages.

PowerShell users can make a disposable copy with:

```powershell
$AskWorkspace = Join-Path ([System.IO.Path]::GetTempPath()) ("colossus-ask-" + [guid]::NewGuid())
Copy-Item -Recurse "examples\asks\fixture" $AskWorkspace
$AskText = Get-Content -Raw "examples\asks\04-failing-test-diagnosis.txt"
& $ColossusBin -w $AskWorkspace --config $ColossusConfig `
  --approval-mode risk-auto run --max-turns 12 $AskText
```

## Exercise Plan and Execute

Run Ask 05 against a fresh fixture copy:

```bash
ASK_TEXT="$(<examples/asks/05-plan-a-fix.txt)"
"$COLOSSUS_BIN" -w "$ASK_WORKSPACE" --config "$COLOSSUS_CONFIG" \
  run --plan "$ASK_TEXT"
"$COLOSSUS_BIN" -w "$ASK_WORKSPACE" --config "$COLOSSUS_CONFIG" plans list
```

Copy the draft plan ID, inspect it, then approve and execute it:

```bash
"$COLOSSUS_BIN" -w "$ASK_WORKSPACE" --config "$COLOSSUS_CONFIG" plans show PLAN_ID
"$COLOSSUS_BIN" -w "$ASK_WORKSPACE" --config "$COLOSSUS_CONFIG" \
  --approval-mode ask plans approve PLAN_ID
"$COLOSSUS_BIN" -w "$ASK_WORKSPACE" --config "$COLOSSUS_CONFIG" \
  --approval-mode ask run --execute-plan PLAN_ID
```

Plan mode must create exactly one durable draft without changing the fixture. Execution
consumes an approved plan once; a second execution attempt must fail closed.

## Exercise subagents

```bash
ASK_TEXT="$(<examples/asks/07-subagent-review.txt)"
"$COLOSSUS_BIN" --config "$COLOSSUS_CONFIG" run "$ASK_TEXT"
"$COLOSSUS_BIN" --config "$COLOSSUS_CONFIG" agents list
```

The primary agent should delegate exactly two workspace-relative, read-only jobs and
synthesize their completed results. If delegation is not advertised by the runtime,
the model should explain the missing capability instead of inventing a result.

## Exercise a skill

Replace `SKILL_NAME` with a configured skill:

```bash
ASK_TEXT="$(<examples/asks/09-active-skill-review.txt)"
"$COLOSSUS_BIN" --config "$COLOSSUS_CONFIG" \
  run --skill SKILL_NAME "$ASK_TEXT"
```

The run should fail before provider execution when the skill is missing or one of its
required tools is unavailable.

## Exercise source-backed research

The research service accepts the question directly:

```bash
ASK_TEXT="$(<examples/asks/10-source-backed-research.txt)"
"$COLOSSUS_BIN" --config "$COLOSSUS_CONFIG" \
  research run --depth standard --source repo --source web "$ASK_TEXT"
```

This example requires a configured search route for `web`. Remove `--source web` for a
repository-only run. A successful report includes stable source labels and claims that
point back to those sources.

## Verify durable evidence

After any example:

```bash
"$COLOSSUS_BIN" --config "$COLOSSUS_CONFIG" sessions list
"$COLOSSUS_BIN" --config "$COLOSSUS_CONFIG" telemetry runs
"$COLOSSUS_BIN" --config "$COLOSSUS_CONFIG" audit verify
```

Use `--output json` before the command for automation. Noninteractive effectful runs
default to denial; approval mode can satisfy an approval obligation but cannot override
a policy denial or add a missing resource grant.
