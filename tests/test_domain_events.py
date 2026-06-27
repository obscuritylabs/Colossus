from pydantic import TypeAdapter

from colossus.domain.events import (
    ApprovalAutoGrantedEvent,
    ModelDeltaEvent,
    ModelRequestPreparedEvent,
    ReasoningSummaryEvent,
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
