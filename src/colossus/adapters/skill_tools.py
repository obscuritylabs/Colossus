"""Model-callable skill authoring tools."""

import json
from typing import Any

from colossus.application.skill_authoring import SkillAuthoringService
from colossus.application.tools import ToolHandler
from colossus.domain.errors import ColossusError, ToolExecutionError
from colossus.domain.tools import ToolPermission, ToolSpec

JsonObject = dict[str, Any]
HandlerMap = dict[str, ToolHandler]


def create_skill_tools(
    service: SkillAuthoringService,
) -> tuple[tuple[ToolSpec, ...], HandlerMap]:
    handlers = SkillToolHandlers(service)
    return (
        (_skill_scaffold_spec(), _skill_validate_spec()),
        {
            "skill.scaffold": handlers.skill_scaffold,
            "skill.validate": handlers.skill_validate,
        },
    )


class SkillToolHandlers:
    def __init__(self, service: SkillAuthoringService) -> None:
        self._service = service

    async def skill_scaffold(self, arguments: JsonObject) -> str:
        try:
            result = self._service.scaffold_user_skill(
                _required_string_arg(arguments, "name"),
                description=_optional_string_arg(arguments, "description"),
                overwrite=_bool_arg(arguments, "overwrite", False),
            )
        except ColossusError as exc:
            raise ToolExecutionError(str(exc)) from exc
        return _json(
            {
                "skill": {
                    "name": result.name,
                    "path": str(result.path),
                    "manifest": result.manifest.model_dump(mode="json"),
                }
            }
        )

    async def skill_validate(self, arguments: JsonObject) -> str:
        try:
            result = self._service.validate_user_skill(_required_string_arg(arguments, "name"))
        except ColossusError as exc:
            raise ToolExecutionError(str(exc)) from exc
        return _json(
            {
                "validation": {
                    "path": str(result.path),
                    "valid": result.valid,
                    "manifest": (
                        result.manifest.model_dump(mode="json")
                        if result.manifest is not None
                        else None
                    ),
                    "errors": list(result.errors),
                }
            }
        )


def _skill_scaffold_spec() -> ToolSpec:
    return ToolSpec(
        name="skill.scaffold",
        description="Create a data-only user skill under the configured skill directory.",
        input_schema=_object_schema(
            {
                "name": {"type": "string"},
                "description": {"type": "string"},
                "overwrite": {"type": "boolean", "default": False},
            },
            ["name"],
        ),
        output_schema=_object_schema({"skill": {"type": "object"}}),
        permissions=ToolPermission(
            filesystem="write",
            approval_required=True,
            mutation=True,
            working_root_required=False,
            risk="high",
        ),
    )


def _skill_validate_spec() -> ToolSpec:
    return ToolSpec(
        name="skill.validate",
        description="Validate a user skill by name under the configured skill directory.",
        input_schema=_object_schema({"name": {"type": "string"}}, ["name"]),
        output_schema=_object_schema({"validation": {"type": "object"}}),
        permissions=ToolPermission(
            filesystem="read",
            working_root_required=False,
            risk="low",
        ),
    )


def _object_schema(
    properties: dict[str, object],
    required: list[str] | None = None,
) -> dict[str, object]:
    return {
        "type": "object",
        "properties": properties,
        "required": required or [],
        "additionalProperties": False,
    }


def _required_string_arg(arguments: JsonObject, key: str) -> str:
    value = arguments.get(key)
    if not isinstance(value, str) or not value.strip():
        raise ToolExecutionError(f"{key} is required.")
    return value


def _optional_string_arg(arguments: JsonObject, key: str) -> str | None:
    value = arguments.get(key)
    if value is None:
        return None
    if not isinstance(value, str):
        raise ToolExecutionError(f"{key} must be a string.")
    return value


def _bool_arg(arguments: JsonObject, key: str, default: bool) -> bool:
    value = arguments.get(key, default)
    if not isinstance(value, bool):
        raise ToolExecutionError(f"{key} must be a boolean.")
    return value


def _json(data: object) -> str:
    return json.dumps(data, sort_keys=True)
