import json

import pytest

from colossus.adapters.skills_filesystem import FilesystemSkillRepository
from colossus.adapters.skills_package import PackageSkillRepository
from colossus.application.defaults import default_agent
from colossus.application.skills import SkillComposer, SkillResolver, extract_skill_mentions
from colossus.domain.errors import ColossusError


def test_bundled_skills_are_discoverable() -> None:
    resolver = SkillResolver((PackageSkillRepository(),))
    names = {skill.manifest.name for skill in resolver.list_skills()}

    assert {"coding", "security-review", "offline-dev"} <= names


def test_bundled_skill_content_loads() -> None:
    skill = PackageSkillRepository().get_skill("coding")

    assert skill is not None
    assert "implementing or changing software" in skill.instructions


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
    path.mkdir()
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
