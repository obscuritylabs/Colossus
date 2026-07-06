"""Model-callable goal-mode tools."""

import json
from typing import Any, cast

from colossus.adapters.tool_schema import injected_argument_schema
from colossus.application.goals import GoalService
from colossus.application.tools import ToolHandler
from colossus.domain.errors import ColossusError, ToolExecutionError
from colossus.domain.goals import Goal, GoalStatus
from colossus.domain.tools import ToolPermission, ToolSpec

JsonObject = dict[str, Any]
HandlerMap = dict[str, ToolHandler]


def create_goal_tools(goal_service: GoalService) -> tuple[tuple[ToolSpec, ...], HandlerMap]:
    handlers = GoalToolHandlers(goal_service)
    return (
        (_goal_show_spec(), _goal_update_spec()),
        {
            "goal.show": handlers.goal_show,
            "goal.update": handlers.goal_update,
        },
    )


class GoalToolHandlers:
    def __init__(self, goal_service: GoalService) -> None:
        self._goal_service = goal_service

    async def goal_show(self, arguments: JsonObject) -> str:
        try:
            goal = await self._goal_service.get_goal(_required_string_arg(arguments, "goal_id"))
        except ColossusError as exc:
            raise ToolExecutionError(str(exc)) from exc
        return _json({"goal": _goal_payload(goal)})

    async def goal_update(self, arguments: JsonObject) -> str:
        try:
            goal = await self._goal_service.update_goal(
                _required_string_arg(arguments, "goal_id"),
                status=_validated_goal_status(_required_string_arg(arguments, "status")),
                summary=_optional_string(arguments, "summary"),
                blocked_reason=_optional_string(arguments, "blocked_reason"),
            )
        except ColossusError as exc:
            raise ToolExecutionError(str(exc)) from exc
        return _json({"goal": _goal_payload(goal)})


def _goal_payload(goal: Goal) -> JsonObject:
    return goal.model_dump(mode="json")


def _goal_statuses() -> set[str]:
    return {"active", "blocked", "complete"}


def _validated_goal_status(value: str) -> GoalStatus:
    if value not in _goal_statuses():
        raise ToolExecutionError("Goal status is not supported.")
    return cast(GoalStatus, value)


def _required_string_arg(arguments: JsonObject, name: str) -> str:
    value = arguments.get(name)
    if not isinstance(value, str) or not value:
        raise ToolExecutionError(f"Argument {name} must be a non-empty string.")
    return value


def _optional_string(arguments: JsonObject, name: str) -> str | None:
    value = arguments.get(name)
    if isinstance(value, str):
        return value
    return None


def _json(value: JsonObject) -> str:
    return json.dumps(value, sort_keys=True)


def _object_schema(properties: JsonObject, required: list[str] | None = None) -> JsonObject:
    return {
        "type": "object",
        "properties": properties,
        "required": required or [],
        "additionalProperties": False,
    }


def _goal_show_spec() -> ToolSpec:
    return ToolSpec(
        name="goal.show",
        description="Show the active goal-mode record.",
        input_schema=_object_schema(
            {"goal_id": injected_argument_schema({"type": "string", "minLength": 1})},
            ["goal_id"],
        ),
        output_schema=_object_schema({"goal": {"type": "object"}}),
        permissions=ToolPermission(working_root_required=False, risk="low"),
    )


def _goal_update_spec() -> ToolSpec:
    return ToolSpec(
        name="goal.update",
        description="Update the active goal-mode status to active, complete, or blocked.",
        input_schema=_object_schema(
            {
                "goal_id": injected_argument_schema({"type": "string", "minLength": 1}),
                "status": {"type": "string", "enum": sorted(_goal_statuses())},
                "summary": {"type": "string"},
                "blocked_reason": {"type": "string"},
            },
            ["goal_id", "status"],
        ),
        output_schema=_object_schema({"goal": {"type": "object"}}),
        permissions=ToolPermission(working_root_required=False, risk="low"),
    )
