import json

import pytest

from colossus.adapters.skills_filesystem import FilesystemSkillRepository
from colossus.adapters.skills_package import PackageSkillRepository
from colossus.application.skills import SkillResolver


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


def _write_skill(path, *, name: str, instructions: str = "instructions") -> None:
    path.mkdir()
    (path / "manifest.json").write_text(
        json.dumps(
            {
                "name": name,
                "version": "1.0.0",
                "description": f"{name} skill",
                "triggers": [name],
                "required_tools": [],
                "permissions": [],
                "offline_compatible": True,
            }
        ),
        encoding="utf-8",
    )
    (path / "SKILL.md").write_text(instructions, encoding="utf-8")
