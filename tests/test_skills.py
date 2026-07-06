import json
import zipfile

import pytest

from colossus.adapters.skills_filesystem import (
    FilesystemSkillRepository,
    WorkspaceSkillRepository,
    workspace_skill_roots,
)
from colossus.adapters.skills_package import PackageSkillRepository
from colossus.application.defaults import default_agent
from colossus.application.skill_authoring import SkillAuthoringService
from colossus.application.skill_loader import load_skill_from_directory
from colossus.application.skills import (
    SkillComposer,
    SkillResolver,
    SkillResourceService,
    extract_skill_mentions,
)
from colossus.domain.errors import ColossusError
from colossus.domain.tools import ToolSpec
from colossus.infrastructure.container import create_default_skill_resolver


def test_bundled_skills_are_discoverable() -> None:
    resolver = SkillResolver((PackageSkillRepository(),))
    names = {skill.manifest.name for skill in resolver.list_skills()}

    assert {"coding", "security-review", "offline-dev", "skill-creator"} <= names


def test_bundled_skill_content_loads() -> None:
    skill = PackageSkillRepository().get_skill("coding")
    creator = PackageSkillRepository().get_skill("skill-creator")

    assert skill is not None
    assert "implementing or changing software" in skill.instructions
    assert creator is not None
    assert "design, write, or revise a Colossus skill" in creator.instructions
    assert "Creation Workflow" in creator.instructions
    assert "Weak Skill Smells" in creator.instructions
    assert "filesystem_read" in creator.instructions
    assert "Colossus canonical dotted tool IDs" in creator.instructions
    assert creator.manifest.version == "0.4.0"


def test_skill_authoring_service_scaffolds_and_validates_skill(tmp_path) -> None:
    service = SkillAuthoringService(tmp_path / "skills")

    result = service.scaffold_user_skill(
        "demo-skill",
        description="Demo workflow.",
        instructions="# Demo Skill\n\nUse this skill for demo workflows.\n",
        triggers=["demo-skill", "demo", "demo"],
        required_tools=["filesystem.read"],
        permissions=["filesystem:read"],
        offline_compatible=False,
    )
    validation = service.validate(result.path)

    assert result.path == tmp_path / "skills" / "demo-skill"
    assert result.manifest.name == "demo-skill"
    assert result.manifest.description == "Demo workflow."
    assert result.manifest.triggers == ("demo-skill", "demo")
    assert result.manifest.required_tools == ("filesystem.read",)
    assert result.manifest.permissions == ("filesystem:read",)
    assert result.manifest.offline_compatible is False
    assert (result.path / "manifest.json").is_file()
    assert (result.path / "SKILL.md").is_file()
    assert (result.path / "SKILL.md").read_text(encoding="utf-8").startswith("# Demo Skill")
    assert validation.valid is True
    assert validation.manifest is not None
    assert validation.manifest.name == "demo-skill"

    with pytest.raises(ColossusError, match="already exists"):
        service.scaffold_user_skill("demo-skill")


def test_skill_authoring_service_defaults_to_workspace_skill_root(tmp_path) -> None:
    service = SkillAuthoringService(
        tmp_path / "data-skills",
        workspace_skill_root=tmp_path / "workspace" / ".agents" / "skills",
    )

    result = service.scaffold("local-skill")

    assert result.path == tmp_path / "workspace" / ".agents" / "skills" / "local-skill"
    assert (result.path / "SKILL.md").is_file()


def test_skill_authoring_service_installs_valid_skill_to_global_root(tmp_path) -> None:
    service = SkillAuthoringService(
        tmp_path / "data-skills",
        workspace_skill_root=tmp_path / "workspace" / ".agents" / "skills",
        global_skill_root=tmp_path / "home" / ".agents" / "skills",
    )
    local = service.scaffold("local-skill", description="Local workflow.")

    installed = service.install_skill(local.path)

    assert installed.name == "local-skill"
    assert installed.target_path == tmp_path / "home" / ".agents" / "skills" / "local-skill"
    assert (installed.target_path / "SKILL.md").is_file()
    assert {file.path for file in installed.files} == {"SKILL.md", "manifest.json"}

    with pytest.raises(ColossusError, match="already exists"):
        service.install_skill(local.path)


def test_skill_authoring_service_reads_and_writes_existing_user_skill(tmp_path) -> None:
    service = SkillAuthoringService(tmp_path / "skills")
    result = service.scaffold_user_skill(
        "demo-skill",
        description="Demo workflow.",
        instructions="# Demo Skill\n\nUse this skill for demo workflows.\n",
        resources=("references",),
    )

    inspected = service.inspect_user_skill("demo-skill")
    read = service.read_user_skill_file("demo-skill", "SKILL.md")
    written = service.write_user_skill_file(
        "demo-skill",
        "SKILL.md",
        "# Demo Skill\n\nUse this skill for stronger demo workflows.\n",
        expected_sha256=read.sha256,
    )
    created_resource = service.write_user_skill_file(
        "demo-skill",
        "references/guide.md",
        "# Guide\n\nReusable guidance.\n",
        mode="create",
    )

    assert inspected.path == result.path
    assert {file.path for file in inspected.files} == {"SKILL.md", "manifest.json"}
    assert read.content.startswith("# Demo Skill")
    assert written.validation.valid is True
    assert written.sha256 != read.sha256
    assert created_resource.path == "references/guide.md"
    assert (result.path / "references" / "guide.md").read_text(encoding="utf-8") == (
        "# Guide\n\nReusable guidance.\n"
    )

    with pytest.raises(ColossusError, match="changed since it was read"):
        service.write_user_skill_file(
            "demo-skill",
            "SKILL.md",
            "# Demo Skill\n\nStale edit.\n",
            expected_sha256=read.sha256,
        )
    with pytest.raises(ColossusError, match="safe relative path"):
        service.read_user_skill_file("demo-skill", "../outside.md")
    with pytest.raises(ColossusError, match=r"must be SKILL\.md"):
        service.write_user_skill_file("demo-skill", "notes.md", "Nope.\n", mode="create")


def test_skill_authoring_service_scaffolds_agent_compatible_resources(tmp_path) -> None:
    service = SkillAuthoringService(tmp_path / "skills")

    result = service.scaffold_user_skill(
        "resource-skill",
        description="Resource workflow.",
        resources=("references", "scripts", "assets"),
        agent_compatible=True,
    )
    skill_text = (result.path / "SKILL.md").read_text(encoding="utf-8")
    validation = service.validate(result.path)

    assert skill_text.startswith("---\nname: resource-skill\n")
    assert "description: Resource workflow." in skill_text
    assert (result.path / "references").is_dir()
    assert (result.path / "scripts").is_dir()
    assert (result.path / "assets").is_dir()
    assert validation.valid is True


def test_skill_authoring_validation_rejects_unsafe_resource_layout(tmp_path) -> None:
    service = SkillAuthoringService(tmp_path / "skills")
    result = service.scaffold_user_skill("bad-resource", resources=("assets",))
    (result.path / "assets" / "image.png").write_bytes(b"\x89PNG")
    (result.path / "extra").mkdir()

    validation = service.validate(result.path)

    assert validation.valid is False
    assert "Unexpected skill directory: extra." in validation.errors
    assert "Resource file should be text or live in scripts/: assets/image.png." in (
        validation.errors
    )


def test_filesystem_skill_repository_loads_frontmatter_only_skill(tmp_path) -> None:
    skill_dir = tmp_path / "agent-skill"
    skill_dir.mkdir()
    (skill_dir / "SKILL.md").write_text(
        "---\n"
        "name: agent-skill\n"
        "description: Agent-compatible skill.\n"
        "---\n\n"
        "# Agent Skill\n\nUse this frontmatter-only skill.\n",
        encoding="utf-8",
    )

    skill = FilesystemSkillRepository(tmp_path).get_skill("agent-skill")

    assert skill is not None
    assert skill.manifest.version == "0.1.0"
    assert skill.manifest.description == "Agent-compatible skill."
    assert skill.manifest.triggers == ("agent-skill", "agent", "skill")
    assert skill.instructions.startswith("# Agent Skill")
    assert skill.resource_root == str(skill_dir.resolve(strict=False))


def test_filesystem_skill_repository_loads_protocol_skill_md_fallback(tmp_path) -> None:
    skill_dir = tmp_path / "agent-skill"
    skill_dir.mkdir()
    (skill_dir / "skill.md").write_text(
        "---\n"
        "name: agent-skill\n"
        "description: Protocol-compatible skill.\n"
        "---\n\n"
        "# Agent Skill\n",
        encoding="utf-8",
    )

    skill = FilesystemSkillRepository(tmp_path).get_skill("agent-skill")

    assert skill is not None
    assert skill.manifest.description == "Protocol-compatible skill."


def test_skill_loader_rejects_duplicate_skill_markdown_names(
    tmp_path,
) -> None:
    archive_path = tmp_path / "skills.zip"
    with zipfile.ZipFile(archive_path, "w") as archive:
        archive.writestr(
            "agent-skill/SKILL.md",
            "---\nname: agent-skill\ndescription: Canonical.\n---\n\n# Agent Skill\n",
        )
        archive.writestr(
            "agent-skill/skill.md",
            "---\nname: agent-skill\ndescription: Protocol.\n---\n\n# Agent Skill\n",
        )

    with zipfile.ZipFile(archive_path) as archive:
        skill_dir = zipfile.Path(archive, "agent-skill/")
        with pytest.raises(ColossusError, match=r"both SKILL\.md and skill\.md"):
            load_skill_from_directory(skill_dir, source="test")


def test_filesystem_skill_repository_rejects_frontmatter_manifest_mismatch(tmp_path) -> None:
    skill_dir = tmp_path / "mismatch"
    _write_skill(skill_dir, name="manifest-name", instructions="# Mismatch\n")
    (skill_dir / "SKILL.md").write_text(
        "---\n"
        "name: frontmatter-name\n"
        "description: manifest-name skill\n"
        "---\n\n"
        "# Mismatch\n",
        encoding="utf-8",
    )

    with pytest.raises(ColossusError, match="does not match"):
        FilesystemSkillRepository(tmp_path).list_skills()


def test_default_skill_resolver_loads_user_skills_and_controls_overrides(tmp_path) -> None:
    _write_skill(tmp_path / "custom", name="custom")
    _write_skill(tmp_path / "coding", name="coding", instructions="User override")

    resolver = create_default_skill_resolver(tmp_path)
    override_resolver = create_default_skill_resolver(tmp_path, allow_user_overrides=True)

    assert resolver.get_skill("custom") is not None
    assert resolver.get_skill("coding").source == "package:coding"  # type: ignore[union-attr]
    assert override_resolver.get_skill("coding").instructions == "User override"  # type: ignore[union-attr]
    duplicate = next(item for item in resolver.duplicate_names() if item.name == "coding")
    override_duplicate = next(
        item for item in override_resolver.duplicate_names() if item.name == "coding"
    )
    assert duplicate.selected_source == "package:coding"
    assert override_duplicate.selected_source == str(tmp_path / "coding")


def test_default_skill_resolver_loads_global_and_workspace_agent_skills(tmp_path) -> None:
    legacy_root = tmp_path / "data" / "skills"
    global_root = tmp_path / "home" / ".agents" / "skills"
    workspace = tmp_path / "repo" / "service"
    (tmp_path / "repo" / ".git").mkdir(parents=True)
    _write_skill(global_root / "global-skill", name="global-skill")
    _write_skill(tmp_path / "repo" / ".agents" / "skills" / "root-skill", name="root-skill")
    _write_skill(workspace / ".agents" / "skills" / "local-skill", name="local-skill")

    resolver = create_default_skill_resolver(
        legacy_root,
        global_skill_root=global_root,
        workspace_root=workspace,
    )

    assert resolver.get_skill("global-skill") is not None
    assert resolver.get_skill("root-skill") is not None
    assert resolver.get_skill("local-skill") is not None
    assert [skill.manifest.name for skill in WorkspaceSkillRepository(workspace).list_skills()] == [
        "root-skill",
        "local-skill",
    ]
    assert workspace_skill_roots(workspace) == (
        tmp_path / "repo" / ".agents" / "skills",
        workspace / ".agents" / "skills",
    )


def test_default_agent_allows_workspace_skills_by_default(tmp_path) -> None:
    _write_skill(tmp_path / ".agents" / "skills" / "alpha", name="alpha")
    composer = SkillComposer(
        create_default_skill_resolver(
            tmp_path / "data" / "skills",
            workspace_root=tmp_path,
            global_skill_root=tmp_path / "home" / ".agents" / "skills",
        )
    )

    composition = composer.compose(
        instructions="Base instructions.",
        agent=default_agent(),
        prompt="@skill:alpha help",
        active_skills=(),
        skill_mode_enabled=True,
        tools=(),
    )

    assert [skill.manifest.name for skill in composition.active_skills] == ["alpha"]


def test_skill_resource_service_restricts_active_text_resources(tmp_path) -> None:
    skill_dir = tmp_path / "resource-skill"
    _write_skill(skill_dir, name="resource-skill")
    (skill_dir / "references").mkdir()
    (skill_dir / "references" / "guide.md").write_text("# Guide\n", encoding="utf-8")
    (skill_dir / "references" / "blob.bin").write_bytes(b"abc\x00def")
    (skill_dir / "references" / "huge.txt").write_text("x" * 64_001, encoding="utf-8")
    resolver = SkillResolver((FilesystemSkillRepository(tmp_path),))
    service = SkillResourceService(resolver)

    resources = service.list_resources(
        skill_name="resource-skill",
        active_skills=("resource-skill",),
    )
    read = service.read_resource(
        skill_name="resource-skill",
        path="references/guide.md",
        active_skills=("resource-skill",),
    )

    assert [resource.path for resource in resources] == [
        "references/blob.bin",
        "references/guide.md",
        "references/huge.txt",
    ]
    assert read.content == "# Guide\n"
    with pytest.raises(ColossusError, match="not active"):
        service.list_resources(skill_name="resource-skill", active_skills=())
    with pytest.raises(ColossusError, match="Invalid skill resource path"):
        service.read_resource(
            skill_name="resource-skill",
            path="../outside.md",
            active_skills=("resource-skill",),
        )
    with pytest.raises(ColossusError, match="not a text-safe"):
        service.read_resource(
            skill_name="resource-skill",
            path="references/blob.bin",
            active_skills=("resource-skill",),
        )
    with pytest.raises(ColossusError, match="too large"):
        service.read_resource(
            skill_name="resource-skill",
            path="references/huge.txt",
            active_skills=("resource-skill",),
        )


def test_filesystem_skill_repository_loads_sorted_skills_and_ignores_incomplete_dirs(
    tmp_path,
) -> None:
    _write_skill(tmp_path / "zeta", name="zeta", instructions="Z instructions")
    _write_skill(tmp_path / "alpha", name="alpha", instructions="A instructions")
    (tmp_path / "draft").mkdir()
    (tmp_path / "draft" / "manifest.json").write_text("{}", encoding="utf-8")

    skills = FilesystemSkillRepository(tmp_path).list_skills()

    assert [skill.manifest.name for skill in skills] == ["alpha", "zeta"]
    assert [skill.instructions for skill in skills] == ["A instructions", "Z instructions"]
    assert all(skill.source.startswith(str(tmp_path)) for skill in skills)


def test_filesystem_skill_repository_honors_disabled_directory_names(tmp_path) -> None:
    _write_skill(tmp_path / "enabled-dir", name="enabled")
    _write_skill(tmp_path / "disabled-dir", name="disabled")

    repository = FilesystemSkillRepository(tmp_path, disabled=frozenset({"disabled-dir"}))

    assert [skill.manifest.name for skill in repository.list_skills()] == ["enabled"]
    assert repository.get_skill("disabled") is None


def test_filesystem_skill_repository_returns_empty_for_missing_root(tmp_path) -> None:
    assert FilesystemSkillRepository(tmp_path / "missing").list_skills() == ()


def test_filesystem_skill_repository_surfaces_invalid_manifest(tmp_path) -> None:
    skill_dir = tmp_path / "broken"
    skill_dir.mkdir()
    (skill_dir / "manifest.json").write_text('{"name":"broken"}', encoding="utf-8")
    (skill_dir / "SKILL.md").write_text("instructions", encoding="utf-8")

    with pytest.raises(ValueError):
        FilesystemSkillRepository(tmp_path).list_skills()


def test_skill_composer_injects_index_and_active_bodies_only(tmp_path) -> None:
    _write_skill(tmp_path / "alpha", name="alpha", instructions="Alpha instructions")
    _write_skill(tmp_path / "beta", name="beta", instructions="Beta instructions")
    composer = SkillComposer(SkillResolver((FilesystemSkillRepository(tmp_path),)))

    composition = composer.compose(
        instructions="Base instructions.",
        agent=default_agent().model_copy(update={"skills": ("alpha", "beta")}),
        prompt="Do the work",
        active_skills=("alpha",),
        skill_mode_enabled=True,
        tools=(),
    )

    assert "[Available skills]" in composition.instructions
    assert "- alpha v1.0.0: alpha skill" in composition.instructions
    assert "- beta v1.0.0: beta skill" in composition.instructions
    assert "Alpha instructions" in composition.instructions
    assert "Beta instructions" not in composition.instructions
    assert [skill.manifest.name for skill in composition.active_skills] == ["alpha"]


def test_skill_composer_activates_mentions_and_dedupes_in_order(tmp_path) -> None:
    _write_skill(tmp_path / "alpha", name="alpha")
    _write_skill(tmp_path / "beta", name="beta")
    composer = SkillComposer(SkillResolver((FilesystemSkillRepository(tmp_path),)))

    composition = composer.compose(
        instructions="Base",
        agent=default_agent().model_copy(update={"skills": ("alpha", "beta")}),
        prompt="@skill:beta then @alpha",
        active_skills=("alpha", "beta"),
        skill_mode_enabled=True,
        tools=(),
    )

    assert [skill.manifest.name for skill in composition.active_skills] == ["alpha", "beta"]


def test_skill_composer_validates_names_and_required_tools(tmp_path) -> None:
    _write_skill(tmp_path / "alpha", name="alpha", required_tools=["echo"])
    _write_skill(tmp_path / "beta", name="beta")
    composer = SkillComposer(SkillResolver((FilesystemSkillRepository(tmp_path),)))
    agent = default_agent().model_copy(update={"skills": ("alpha", "beta")})

    with pytest.raises(ColossusError, match="Unknown skill"):
        composer.compose(
            instructions="Base",
            agent=agent,
            prompt="@skill:missing",
            active_skills=(),
            skill_mode_enabled=True,
            tools=(),
        )
    with pytest.raises(ColossusError, match="not available"):
        composer.compose(
            instructions="Base",
            agent=agent.model_copy(update={"skills": ("beta",)}),
            prompt="",
            active_skills=("alpha",),
            skill_mode_enabled=True,
            tools=(),
        )
    with pytest.raises(ColossusError, match="requires unavailable tools"):
        composer.compose(
            instructions="Base",
            agent=agent,
            prompt="",
            active_skills=("alpha",),
            skill_mode_enabled=True,
            tools=(),
        )


def test_skill_composer_suggests_dotted_tool_names_for_provider_safe_names(
    tmp_path,
) -> None:
    _write_skill(
        tmp_path / "alpha",
        name="alpha",
        required_tools=["filesystem_read", "filesystem_write"],
    )
    composer = SkillComposer(SkillResolver((FilesystemSkillRepository(tmp_path),)))
    agent = default_agent().model_copy(update={"skills": ("alpha",)})
    tools = (
        ToolSpec(name="filesystem.read", description="Read files.", input_schema={}),
        ToolSpec(name="filesystem.write", description="Write files.", input_schema={}),
    )

    with pytest.raises(ColossusError) as exc_info:
        composer.compose(
            instructions="Base",
            agent=agent,
            prompt="",
            active_skills=("alpha",),
            skill_mode_enabled=True,
            tools=tools,
        )

    message = str(exc_info.value)
    assert "filesystem_read (did you mean filesystem.read?)" in message
    assert "filesystem_write (did you mean filesystem.write?)" in message


def test_skill_composer_disabled_mode_blocks_mentions(tmp_path) -> None:
    _write_skill(tmp_path / "alpha", name="alpha")
    composer = SkillComposer(SkillResolver((FilesystemSkillRepository(tmp_path),)))

    with pytest.raises(ColossusError, match="Skill Mode is disabled"):
        composer.compose(
            instructions="Base",
            agent=default_agent().model_copy(update={"skills": ("alpha",)}),
            prompt="@skill:alpha",
            active_skills=(),
            skill_mode_enabled=False,
            tools=(),
        )


def test_skill_mentions_accept_canonical_and_known_shorthand() -> None:
    assert extract_skill_mentions(
        "Use @skill:coding and @offline-dev but not user@example.com.",
        available_names=("coding", "offline-dev"),
    ) == ("coding", "offline-dev")


def _write_skill(
    path,
    *,
    name: str,
    instructions: str = "instructions",
    required_tools: list[str] | None = None,
) -> None:
    path.mkdir(parents=True)
    (path / "manifest.json").write_text(
        json.dumps(
            {
                "name": name,
                "version": "1.0.0",
                "description": f"{name} skill",
                "triggers": [name],
                "required_tools": required_tools or [],
                "permissions": [],
                "offline_compatible": True,
            }
        ),
        encoding="utf-8",
    )
    (path / "SKILL.md").write_text(instructions, encoding="utf-8")
