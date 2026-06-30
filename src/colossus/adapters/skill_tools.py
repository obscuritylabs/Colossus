"""Model-callable skill authoring tools."""

import json
from pathlib import Path
from typing import Any

from colossus.application.skill_authoring import SkillAuthoringService, SkillValidationResult
from colossus.application.skills import SkillResourceService
from colossus.application.tools import ToolHandler
from colossus.domain.errors import ColossusError, ToolExecutionError
from colossus.domain.tools import ToolPermission, ToolSpec
from colossus.ports.audit import AuditSink

JsonObject = dict[str, Any]
HandlerMap = dict[str, ToolHandler]


def create_skill_tools(
    service: SkillAuthoringService,
    resource_service: SkillResourceService | None = None,
    audit_sink: AuditSink | None = None,
) -> tuple[tuple[ToolSpec, ...], HandlerMap]:
    handlers = SkillToolHandlers(service, resource_service, audit_sink=audit_sink)
    resource_specs = (
        (_skill_resource_list_spec(), _skill_resource_read_spec())
        if resource_service is not None
        else ()
    )
    resource_handlers: HandlerMap = (
        {
            "skill.resource.list": handlers.skill_resource_list,
            "skill.resource.read": handlers.skill_resource_read,
        }
        if resource_service is not None
        else {}
    )
    return (
        (
            _skill_scaffold_spec(),
            _skill_inspect_spec(),
            _skill_read_spec(),
            _skill_write_spec(),
            _skill_validate_spec(),
            _skill_install_spec(),
            *resource_specs,
        ),
        {
            "skill.scaffold": handlers.skill_scaffold,
            "skill.inspect": handlers.skill_inspect,
            "skill.read": handlers.skill_read,
            "skill.write": handlers.skill_write,
            "skill.validate": handlers.skill_validate,
            "skill.install": handlers.skill_install,
            **resource_handlers,
        },
    )


class SkillToolHandlers:
    def __init__(
        self,
        service: SkillAuthoringService,
        resource_service: SkillResourceService | None = None,
        audit_sink: AuditSink | None = None,
    ) -> None:
        self._service = service
        self._resource_service = resource_service
        self._audit_sink = audit_sink

    async def skill_scaffold(self, arguments: JsonObject) -> str:
        try:
            result = self._service.scaffold_user_skill(
                _required_string_arg(arguments, "name"),
                description=_optional_string_arg(arguments, "description"),
                instructions=_optional_string_arg(arguments, "instructions"),
                triggers=_optional_string_list_arg(arguments, "triggers"),
                required_tools=_optional_string_list_arg(arguments, "required_tools"),
                permissions=_optional_string_list_arg(arguments, "permissions"),
                offline_compatible=_bool_arg(arguments, "offline_compatible", True),
                resources=_optional_string_list_arg(arguments, "resources"),
                agent_compatible=_bool_arg(arguments, "agent_compatible", False),
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

    async def skill_inspect(self, arguments: JsonObject) -> str:
        try:
            result = self._service.inspect_user_skill(_required_string_arg(arguments, "name"))
        except ColossusError as exc:
            raise ToolExecutionError(str(exc)) from exc
        return _json(
            {
                "skill": {
                    "name": result.name,
                    "path": str(result.path),
                    "files": [
                        {
                            "path": file.path,
                            "size": file.size,
                            "sha256": file.sha256,
                        }
                        for file in result.files
                    ],
                    "truncated": result.truncated,
                    "validation": _validation_payload(result.validation),
                }
            }
        )

    async def skill_read(self, arguments: JsonObject) -> str:
        try:
            result = self._service.read_user_skill_file(
                _required_string_arg(arguments, "name"),
                _string_arg(arguments, "path", "SKILL.md"),
            )
        except ColossusError as exc:
            raise ToolExecutionError(str(exc)) from exc
        if self._audit_sink is not None:
            await self._audit_sink.record(
                "agent",
                "skill.read",
                {
                    "skill": result.name,
                    "path": result.path,
                    "size": result.size,
                    "sha256": result.sha256,
                },
            )
        return _json(
            {
                "file": {
                    "skill": result.name,
                    "path": result.path,
                    "size": result.size,
                    "sha256": result.sha256,
                    "content": result.content,
                }
            }
        )

    async def skill_write(self, arguments: JsonObject) -> str:
        try:
            result = self._service.write_user_skill_file(
                _required_string_arg(arguments, "name"),
                _required_string_arg(arguments, "path"),
                _required_string_arg(arguments, "content"),
                mode=_string_arg(arguments, "mode", "overwrite"),
                expected_sha256=_optional_string_arg(arguments, "expected_sha256"),
            )
        except ColossusError as exc:
            raise ToolExecutionError(str(exc)) from exc
        if self._audit_sink is not None:
            await self._audit_sink.record(
                "agent",
                "skill.write",
                {
                    "skill": result.name,
                    "path": result.path,
                    "size": result.size,
                    "sha256": result.sha256,
                    "mode": result.mode,
                    "valid": result.validation.valid,
                },
            )
        return _json(
            {
                "file": {
                    "skill": result.name,
                    "path": result.path,
                    "size": result.size,
                    "sha256": result.sha256,
                    "mode": result.mode,
                    "validation": _validation_payload(result.validation),
                }
            }
        )

    async def skill_resource_list(self, arguments: JsonObject) -> str:
        if self._resource_service is None:
            raise ToolExecutionError("skill.resource.list is unavailable.")
        try:
            result = self._resource_service.list_resources(
                skill_name=_required_string_arg(arguments, "skill"),
                active_skills=_required_string_list_arg(arguments, "active_skills"),
            )
        except ColossusError as exc:
            raise ToolExecutionError(str(exc)) from exc
        return _json({"resources": [item.model_dump(mode="json") for item in result]})

    async def skill_resource_read(self, arguments: JsonObject) -> str:
        if self._resource_service is None:
            raise ToolExecutionError("skill.resource.read is unavailable.")
        skill_name = _required_string_arg(arguments, "skill")
        path = _required_string_arg(arguments, "path")
        active_skills = _required_string_list_arg(arguments, "active_skills")
        try:
            result = self._resource_service.read_resource(
                skill_name=skill_name,
                path=path,
                active_skills=active_skills,
            )
        except ColossusError as exc:
            raise ToolExecutionError(str(exc)) from exc
        if self._audit_sink is not None:
            await self._audit_sink.record(
                "agent",
                "skill.resource.read",
                {
                    "skill": skill_name,
                    "path": result.path,
                    "size": result.size,
                },
            )
        return _json({"resource": result.model_dump(mode="json")})

    async def skill_validate(self, arguments: JsonObject) -> str:
        try:
            path = _optional_string_arg(arguments, "path")
            name = _optional_string_arg(arguments, "name")
            if path is not None:
                result = self._service.validate(Path(path))
            elif name is not None:
                result = self._service.validate_user_skill(name)
            else:
                raise ToolExecutionError("name or path is required.")
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

    async def skill_install(self, arguments: JsonObject) -> str:
        try:
            result = self._service.install_skill(
                Path(_required_string_arg(arguments, "source_path")),
                overwrite=_bool_arg(arguments, "overwrite", False),
            )
        except ColossusError as exc:
            raise ToolExecutionError(str(exc)) from exc
        if self._audit_sink is not None:
            await self._audit_sink.record(
                "agent",
                "skill.install",
                {
                    "skill": result.name,
                    "source_path": str(result.source_path),
                    "target_path": str(result.target_path),
                    "files": [
                        {"path": file.path, "size": file.size, "sha256": file.sha256}
                        for file in result.files
                    ],
                    "valid": result.validation.valid,
                },
            )
        return _json(
            {
                "skill": {
                    "name": result.name,
                    "source_path": str(result.source_path),
                    "target_path": str(result.target_path),
                    "files": [
                        {"path": file.path, "size": file.size, "sha256": file.sha256}
                        for file in result.files
                    ],
                    "validation": _validation_payload(result.validation),
                }
            }
        )


def _skill_scaffold_spec() -> ToolSpec:
    return ToolSpec(
        name="skill.scaffold",
        description=(
            "Create or overwrite a data-only user skill under the configured skill "
            "directory, including manifest fields and SKILL.md instructions."
        ),
        input_schema=_object_schema(
            {
                "name": {"type": "string"},
                "description": {"type": "string"},
                "instructions": {"type": "string", "maxLength": 60000},
                "triggers": {"type": "array", "items": {"type": "string"}},
                "required_tools": {"type": "array", "items": {"type": "string"}},
                "permissions": {"type": "array", "items": {"type": "string"}},
                "offline_compatible": {"type": "boolean", "default": True},
                "resources": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["assets", "examples", "references", "scripts", "tests"],
                    },
                },
                "agent_compatible": {"type": "boolean", "default": False},
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


def _skill_inspect_spec() -> ToolSpec:
    return ToolSpec(
        name="skill.inspect",
        description=(
            "Inspect an existing user skill under the configured skill directory, "
            "including files, hashes, and validation status."
        ),
        input_schema=_object_schema({"name": {"type": "string"}}, ["name"]),
        output_schema=_object_schema({"skill": {"type": "object"}}),
        permissions=ToolPermission(
            filesystem="read",
            working_root_required=False,
            risk="low",
        ),
        max_output_bytes=80_000,
    )


def _skill_read_spec() -> ToolSpec:
    return ToolSpec(
        name="skill.read",
        description=(
            "Read SKILL.md, manifest.json, or a bounded UTF-8 text file under an "
            "existing user skill resource directory."
        ),
        input_schema=_object_schema(
            {
                "name": {"type": "string"},
                "path": {"type": "string", "default": "SKILL.md"},
            },
            ["name"],
        ),
        output_schema=_object_schema({"file": {"type": "object"}}),
        permissions=ToolPermission(
            filesystem="read",
            working_root_required=False,
            risk="low",
        ),
        max_output_bytes=100_000,
    )


def _skill_write_spec() -> ToolSpec:
    return ToolSpec(
        name="skill.write",
        description=(
            "Create or overwrite a bounded UTF-8 text file inside an existing user "
            "skill. Paths are restricted to SKILL.md, manifest.json, and allowed "
            "resource directories. Overwriting existing files requires expected_sha256 "
            "from skill.read or skill.inspect."
        ),
        input_schema=_object_schema(
            {
                "name": {"type": "string"},
                "path": {"type": "string"},
                "content": {"type": "string", "maxLength": 80000},
                "mode": {
                    "type": "string",
                    "enum": ["create", "overwrite"],
                    "default": "overwrite",
                },
                "expected_sha256": {"type": "string"},
            },
            ["name", "path", "content"],
        ),
        output_schema=_object_schema({"file": {"type": "object"}}),
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
        description=(
            "Validate an installed user skill by name or a local skill directory by path."
        ),
        input_schema=_object_schema(
            {"name": {"type": "string"}, "path": {"type": "string"}}
        ),
        output_schema=_object_schema({"validation": {"type": "object"}}),
        permissions=ToolPermission(
            filesystem="read",
            working_root_required=False,
            risk="low",
        ),
    )


def _skill_install_spec() -> ToolSpec:
    return ToolSpec(
        name="skill.install",
        description=(
            "Validate and install a local skill directory into the user-global "
            "~/.agents/skills directory. Existing global skills require overwrite=true."
        ),
        input_schema=_object_schema(
            {
                "source_path": {"type": "string"},
                "overwrite": {"type": "boolean", "default": False},
            },
            ["source_path"],
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


def _skill_resource_list_spec() -> ToolSpec:
    return ToolSpec(
        name="skill.resource.list",
        description=(
            "List files under references, scripts, assets, examples, or tests "
            "for an active skill."
        ),
        input_schema=_object_schema(
            {
                "skill": {"type": "string"},
                "active_skills": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Injected by Colossus.",
                },
            },
            ["skill", "active_skills"],
        ),
        output_schema=_object_schema({"resources": {"type": "array"}}),
        permissions=ToolPermission(
            filesystem="read",
            working_root_required=False,
            risk="low",
        ),
    )


def _skill_resource_read_spec() -> ToolSpec:
    return ToolSpec(
        name="skill.resource.read",
        description="Read a bounded text resource from an active skill.",
        input_schema=_object_schema(
            {
                "skill": {"type": "string"},
                "path": {"type": "string"},
                "active_skills": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Injected by Colossus.",
                },
            },
            ["skill", "path", "active_skills"],
        ),
        output_schema=_object_schema({"resource": {"type": "object"}}),
        permissions=ToolPermission(
            filesystem="read",
            working_root_required=False,
            risk="low",
        ),
        max_output_bytes=80_000,
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


def _string_arg(arguments: JsonObject, key: str, default: str) -> str:
    value = arguments.get(key, default)
    if not isinstance(value, str) or not value.strip():
        raise ToolExecutionError(f"{key} must be a non-empty string.")
    return value


def _optional_string_list_arg(arguments: JsonObject, key: str) -> tuple[str, ...] | None:
    value = arguments.get(key)
    if value is None:
        return None
    if not isinstance(value, list):
        raise ToolExecutionError(f"{key} must be an array of strings.")
    items: list[str] = []
    for item in value:
        if not isinstance(item, str):
            raise ToolExecutionError(f"{key} must be an array of strings.")
        items.append(item)
    return tuple(items)


def _required_string_list_arg(arguments: JsonObject, key: str) -> tuple[str, ...]:
    value = _optional_string_list_arg(arguments, key)
    if value is None:
        raise ToolExecutionError(f"{key} is required.")
    return value


def _bool_arg(arguments: JsonObject, key: str, default: bool) -> bool:
    value = arguments.get(key, default)
    if not isinstance(value, bool):
        raise ToolExecutionError(f"{key} must be a boolean.")
    return value


def _json(data: object) -> str:
    return json.dumps(data, sort_keys=True)


def _validation_payload(result: SkillValidationResult) -> JsonObject:
    return {
        "path": str(result.path),
        "valid": result.valid,
        "manifest": (
            result.manifest.model_dump(mode="json") if result.manifest is not None else None
        ),
        "errors": list(result.errors),
    }
