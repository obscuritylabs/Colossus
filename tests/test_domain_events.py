from pydantic import TypeAdapter

from colossus.domain.events import (
    ApprovalAutoGrantedEvent,
    ContextPreparedEvent,
    ModelDeltaEvent,
    ModelRequestPreparedEvent,
    ReasoningSummaryEvent,
    ResearchProgressEvent,
    RiskAssessmentEvent,
    RunEvent,
    SubagentStatusEvent,
)


def test_run_event_round_trips_with_discriminator() -> None:
    adapter: TypeAdapter[RunEvent] = TypeAdapter(RunEvent)
    event = ModelDeltaEvent(text="hello")

    payload = adapter.dump_json(event)
    parsed = adapter.validate_json(payload)

    assert isinstance(parsed, ModelDeltaEvent)
    assert parsed.text == "hello"


def test_risk_assessment_event_round_trips_with_discriminator() -> None:
    adapter: TypeAdapter[RunEvent] = TypeAdapter(RunEvent)
    event = RiskAssessmentEvent(
        call_id="call-1",
        tool="shell.run",
        risk_level="medium",
        summary="Needs review.",
        recommended_decision="requires_approval",
        model_role="risk_evaluator",
        profile_name="risk",
    )

    parsed = adapter.validate_json(adapter.dump_json(event))

    assert isinstance(parsed, RiskAssessmentEvent)
    assert parsed.recommended_decision == "requires_approval"


def test_reasoning_summary_event_round_trips_with_discriminator() -> None:
    adapter: TypeAdapter[RunEvent] = TypeAdapter(RunEvent)
    event = ReasoningSummaryEvent(
        summary="Checked the next safe action.",
        provider_format="openrouter",
        detail_id="detail-1",
    )

    parsed = adapter.validate_json(adapter.dump_json(event))

    assert isinstance(parsed, ReasoningSummaryEvent)
    assert parsed.summary == "Checked the next safe action."
    assert parsed.provider_format == "openrouter"


def test_model_request_prepared_event_round_trips_with_discriminator() -> None:
    adapter: TypeAdapter[RunEvent] = TypeAdapter(RunEvent)
    event = ModelRequestPreparedEvent(
        turn=0,
        model="demo",
        instructions="system prompt",
        messages=({"role": "user", "content": "hello"},),
        tools=({"name": "filesystem.read", "description": "Read files"},),
    )

    parsed = adapter.validate_json(adapter.dump_json(event))

    assert isinstance(parsed, ModelRequestPreparedEvent)
    assert parsed.instructions == "system prompt"
    assert parsed.messages[0]["content"] == "hello"


def test_context_prepared_event_round_trips_with_discriminator() -> None:
    adapter: TypeAdapter[RunEvent] = TypeAdapter(RunEvent)
    event = ContextPreparedEvent(
        turn=0,
        model="demo",
        token_estimate=1_200,
        original_token_estimate=12_000,
        context_window_tokens=16_000,
        threshold_tokens=11_200,
        target_tokens=7_200,
        snapshot_id="snapshot-1",
        compacted=True,
        snapshot_created=True,
    )

    parsed = adapter.validate_json(adapter.dump_json(event))

    assert isinstance(parsed, ContextPreparedEvent)
    assert parsed.compacted is True
    assert parsed.snapshot_id == "snapshot-1"
    assert parsed.snapshot_created is True


def test_approval_auto_granted_event_round_trips_with_discriminator() -> None:
    adapter: TypeAdapter[RunEvent] = TypeAdapter(RunEvent)
    event = ApprovalAutoGrantedEvent(call_id="call-1", reason="Low-risk command.")

    parsed = adapter.validate_json(adapter.dump_json(event))

    assert isinstance(parsed, ApprovalAutoGrantedEvent)
    assert parsed.reason == "Low-risk command."


def test_subagent_status_event_round_trips_with_discriminator() -> None:
    adapter: TypeAdapter[RunEvent] = TypeAdapter(RunEvent)
    event = SubagentStatusEvent(
        job_id="agent-1",
        status="queued",
        role="subagent_default",
        task="Check the tests.",
        message="Subagent job queued.",
    )

    parsed = adapter.validate_json(adapter.dump_json(event))

    assert isinstance(parsed, SubagentStatusEvent)
    assert parsed.job_id == "agent-1"


def test_research_progress_event_round_trips_with_discriminator() -> None:
    adapter: TypeAdapter[RunEvent] = TypeAdapter(RunEvent)
    event = ResearchProgressEvent(
        research_id="research-1",
        phase="collecting",
        action="web",
        status="completed",
        message="Web search returned 4 result(s).",
        query="deep research progress telemetry",
        source_kind="web",
        current=1,
        total=3,
        sources_collected=4,
        claims_collected=0,
        details={"results": 4, "added": 4, "approved": True},
    )

    parsed = adapter.validate_json(adapter.dump_json(event))

    assert isinstance(parsed, ResearchProgressEvent)
    assert parsed.type == "research.progress"
    assert parsed.source_kind == "web"
    assert parsed.details["results"] == 4
