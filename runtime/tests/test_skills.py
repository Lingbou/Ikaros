from __future__ import annotations

import os
from pathlib import Path
from typing import cast

import pytest

from ikaros_runtime.agent.scheduler import AgentScheduler
from ikaros_runtime.bootstrap import RuntimeApplication
from ikaros_runtime.config import ConfigDocumentStore
from ikaros_runtime.domain import JournalEvent, SkillDescriptor, WorkspaceSummary
from ikaros_runtime.errors import ConfigError, InvalidParamsError
from ikaros_runtime.identity import load_identity_core
from ikaros_runtime.memory import SqliteMemoryStore
from ikaros_runtime.providers.registry import ConfigStore
from ikaros_runtime.services.skills import SkillService
from ikaros_runtime.services.turns import TurnService
from ikaros_runtime.skills import SkillCatalog, build_skill_prompt
from ikaros_runtime.skills.catalog import MAX_FRONTMATTER_BYTES, MAX_SKILL_BYTES
from ikaros_runtime.storage import SqliteRuntimeStore

from .helpers import prepare_turn


def _write_skill(
    root: Path,
    name: str,
    description: str = "A useful local Skill.",
    *,
    extra_frontmatter: str = "",
    body: str = "Follow these instructions.",
) -> Path:
    directory = root / name
    directory.mkdir(parents=True, exist_ok=True)
    suffix = f"\n{extra_frontmatter}" if extra_frontmatter else ""
    path = directory / "SKILL.md"
    path.write_text(
        f"---\nname: {name}\ndescription: {description}{suffix}\n---\n\n{body}\n",
        encoding="utf-8",
    )
    return path


def test_catalog_discovers_sorted_metadata_only_and_ignores_extra_frontmatter(
    tmp_path: Path,
) -> None:
    root = tmp_path / "skills"
    second = _write_skill(root, "zeta", "Last", body="secret body instructions")
    first = _write_skill(root, "alpha", "First", extra_frontmatter="license: MIT")
    (root / "README.txt").write_text("not a Skill", encoding="utf-8")

    discovery = SkillCatalog(root).discover()

    assert discovery.diagnostics == ()
    assert [skill.name for skill in discovery.skills] == ["alpha", "zeta"]
    assert discovery.skills[0].description == "First"
    assert discovery.skills[0].location == str(first.resolve())
    assert discovery.skills[1].location == str(second.resolve())
    assert "secret body instructions" not in repr(discovery.skills)


def test_missing_catalog_is_empty_without_creating_it(tmp_path: Path) -> None:
    root = tmp_path / "skills"

    assert SkillCatalog(root).discover().skills == ()
    assert not root.exists()


@pytest.mark.parametrize(
    ("directory", "source", "code"),
    [
        ("Bad_Name", "---\nname: Bad_Name\ndescription: Test\n---\n", "invalid_name"),
        ("demo", "---\nname: other\ndescription: Test\n---\n", "name_mismatch"),
        ("demo", "---\nname: demo\ndescription: ''\n---\n", "invalid_description"),
        ("demo", "name: demo\ndescription: Test\n", "invalid_frontmatter"),
        (
            "demo",
            "---\nname: demo\ndescription: one\ndescription: two\n---\n",
            "invalid_frontmatter",
        ),
    ],
)
def test_bad_skill_is_diagnostic_and_does_not_block_valid_skills(
    tmp_path: Path,
    directory: str,
    source: str,
    code: str,
) -> None:
    root = tmp_path / "skills"
    _write_skill(root, "valid")
    bad = root / directory
    bad.mkdir(parents=True, exist_ok=True)
    (bad / "SKILL.md").write_text(source, encoding="utf-8")

    discovery = SkillCatalog(root).discover()

    assert [skill.name for skill in discovery.skills] == ["valid"]
    assert [(item.entry, item.code) for item in discovery.diagnostics] == [(directory, code)]


def test_catalog_rejects_invalid_utf8_and_bounded_inputs(tmp_path: Path) -> None:
    root = tmp_path / "skills"
    invalid = root / "invalid-utf8"
    invalid.mkdir(parents=True)
    (invalid / "SKILL.md").write_bytes(b"---\nname: invalid-utf8\ndescription: \xff\n---\n")
    huge = root / "huge"
    huge.mkdir()
    (huge / "SKILL.md").write_bytes(b"x" * (MAX_SKILL_BYTES + 1))
    frontmatter = root / "frontmatter"
    frontmatter.mkdir()
    (frontmatter / "SKILL.md").write_text(
        "---\nname: frontmatter\ndescription: Test\nignored: "
        + ("x" * MAX_FRONTMATTER_BYTES)
        + "\n---\n",
        encoding="utf-8",
    )

    diagnostics = {item.entry: item.code for item in SkillCatalog(root).discover().diagnostics}

    assert diagnostics == {
        "frontmatter": "frontmatter_too_large",
        "huge": "file_too_large",
        "invalid-utf8": "invalid_utf8",
    }


def test_missing_file_and_link_like_paths_are_diagnostics(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    root = tmp_path / "skills"
    (root / "missing").mkdir(parents=True)
    _write_skill(root, "unsafe")
    from ikaros_runtime.skills import catalog as catalog_module

    original = catalog_module._is_link_like
    monkeypatch.setattr(
        catalog_module,
        "_is_link_like",
        lambda path: path.name == "unsafe" or original(path),
    )

    diagnostics = {item.entry: item.code for item in SkillCatalog(root).discover().diagnostics}

    assert diagnostics == {"missing": "missing_file", "unsafe": "unsafe_path"}


def test_real_directory_symlink_is_never_followed_when_supported(tmp_path: Path) -> None:
    root = tmp_path / "skills"
    _write_skill(root, "target")
    link = root / "linked"
    try:
        os.symlink(root / "target", link, target_is_directory=True)
    except OSError:
        pytest.skip("directory symlinks are unavailable on this host")

    discovery = SkillCatalog(root).discover()

    assert [skill.name for skill in discovery.skills] == ["target"]
    assert ("linked", "unsafe_path") in {
        (item.entry, item.code) for item in discovery.diagnostics
    }


def test_catalog_redacts_protected_metadata_and_os_errors(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    root = tmp_path / "skills"
    _write_skill(root, "protected", "Contains sk-secret-sentinel")
    _write_skill(root, "failed")
    from ikaros_runtime.skills import catalog as catalog_module

    original = catalog_module._read_bounded

    def fail_one(path: Path) -> bytes:
        if path.parent.name == "failed":
            raise OSError("sk-secret-sentinel must not escape")
        return original(path)

    monkeypatch.setattr(catalog_module, "_read_bounded", fail_one)
    discovery = SkillCatalog(root, lambda: ("sk-secret-sentinel",)).discover()

    rendered = repr(discovery)
    assert "sk-secret-sentinel" not in rendered
    assert {(item.entry, item.code) for item in discovery.diagnostics} == {
        ("failed", "read_failed"),
        ("<redacted>", "protected_value"),
    }


def test_skill_prompt_is_xml_escaped_and_never_contains_skill_body(tmp_path: Path) -> None:
    root = tmp_path / "skills"
    _write_skill(root, "demo", "Use A & B <carefully>", body="body-must-stay-lazy")
    descriptor = SkillCatalog(root).discover().skills[0]

    prompt = build_skill_prompt((descriptor,))

    assert prompt is not None
    assert "Use A &amp; B &lt;carefully&gt;" in prompt
    assert "<name>demo</name>" in prompt
    assert "use the read tool" in prompt
    assert "body-must-stay-lazy" not in prompt
    assert build_skill_prompt(()) is None


def test_service_lists_toggles_and_preserves_provider_section(tmp_path: Path) -> None:
    root = tmp_path / "skills"
    _write_skill(root, "alpha")
    _write_skill(root, "beta")
    document = ConfigDocumentStore(tmp_path)
    provider_section = {"local": {"api_key": "sk-kept", "models": {"model": {}}}}
    document.replace_section("providers", provider_section)
    service = SkillService(SkillCatalog(root), document)

    initial = service.list_skills({})
    skills = cast(list[dict[str, object]], initial["skills"])
    assert [(item["name"], item["enabled"]) for item in skills] == [
        ("alpha", True),
        ("beta", True),
    ]
    toggled = cast(
        dict[str, object],
        service.set_enabled({"name": "beta", "enabled": False})["skill"],
    )
    assert toggled["enabled"] is False
    assert document.read_section("skills") == {"disabled": ["beta"]}
    assert document.read_section("providers") == provider_section
    assert [item.name for item in service.enabled_descriptors()] == ["alpha"]

    service.set_enabled({"name": "beta", "enabled": True})

    assert document.read_section("skills") is None
    assert document.read_section("providers") == provider_section


@pytest.mark.parametrize(
    "section",
    [
        {"unknown": []},
        {"disabled": "alpha"},
        {"disabled": ["alpha", "alpha"]},
        {"disabled": ["Bad_Name"]},
    ],
)
def test_service_rejects_invalid_skills_configuration(
    tmp_path: Path,
    section: dict[str, object],
) -> None:
    document = ConfigDocumentStore(tmp_path)
    document.replace_section("skills", section)

    with pytest.raises(ConfigError):
        SkillService(SkillCatalog(tmp_path / "skills"), document)


def test_service_rejects_bad_params_and_missing_skills(tmp_path: Path) -> None:
    service = SkillService(SkillCatalog(tmp_path / "skills"), ConfigDocumentStore(tmp_path))

    with pytest.raises(InvalidParamsError, match="does not accept"):
        service.list_skills({"refresh": True})
    with pytest.raises(InvalidParamsError, match="exactly"):
        service.set_enabled({"name": "missing"})
    with pytest.raises(InvalidParamsError, match="boolean"):
        service.set_enabled({"name": "missing", "enabled": 1})
    with pytest.raises(InvalidParamsError, match="does not exist"):
        service.set_enabled({"name": "missing", "enabled": False})


def test_run_skill_snapshot_is_journaled_rebuilt_and_reopened(tmp_path: Path) -> None:
    skill_file = _write_skill(tmp_path / "skills", "demo", "Frozen description")
    descriptor = SkillDescriptor("demo", "Frozen description", str(skill_file.resolve()))
    database_path = tmp_path / "state.db"
    store = SqliteRuntimeStore(database_path)
    try:
        ordinary, _ = store.create_thread("Ordinary")
        workspace, _ = store.create_thread(
            "Workspace",
            workspace=WorkspaceSummary("workspace", "Workspace", str(tmp_path)),
        )
        prepared = prepare_turn(
            store,
            thread_id=ordinary.id,
            branch_id=ordinary.default_branch_id,
            content="Use the Skill",
            provider_id="scripted",
            model_id="scripted-v1",
            skills=(descriptor,),
        )
        workspace_prepared = prepare_turn(
            store,
            thread_id=workspace.id,
            branch_id=workspace.default_branch_id,
            content="Use the same Skill",
            provider_id="scripted",
            model_id="scripted-v1",
            skills=(descriptor,),
        )

        assert prepared.initial_events[0].payload["run"]["skills"] == [descriptor.to_wire()]
        assert store.get_run(prepared.run_id).skills == (descriptor,)
        assert store.get_run(workspace_prepared.run_id).skills == (descriptor,)
        store.rebuild_projections()
        assert store.get_run(prepared.run_id).skills == (descriptor,)
    finally:
        store.close()

    reopened = SqliteRuntimeStore(database_path)
    try:
        assert reopened.get_run(prepared.run_id).skills == (descriptor,)
    finally:
        reopened.close()


def test_prepare_turn_canonicalizes_unsorted_and_rejects_duplicate_skill_snapshots(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Skills")
        alpha = SkillDescriptor("alpha", "Alpha", str((tmp_path / "alpha.md").resolve()))
        beta = SkillDescriptor("beta", "Beta", str((tmp_path / "beta.md").resolve()))

        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="Unsorted",
            provider_id="scripted",
            model_id="scripted-v1",
            skills=(beta, alpha),
        )
        assert store.get_run(prepared.run_id).skills == (alpha, beta)

        with pytest.raises(ValueError, match="Skill names must be unique"):
            prepare_turn(
                store,
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                content="Duplicate",
                provider_id="scripted",
                model_id="scripted-v1",
                skills=(alpha, alpha),
            )
    finally:
        store.close()


class _ReserveOnlyScheduler:
    def __init__(self) -> None:
        self.reserved: list[str] = []

    def reserve(self, run_id: str) -> None:
        self.reserved.append(run_id)


def test_client_request_retry_returns_before_rescanning_skills(tmp_path: Path) -> None:
    root = tmp_path / "skills"
    skill_file = _write_skill(root, "demo")
    descriptor = SkillDescriptor("demo", "A useful local Skill.", str(skill_file.resolve()))
    store = SqliteRuntimeStore(tmp_path / "state.db")
    scheduler = _ReserveOnlyScheduler()
    scans = 0

    def snapshot() -> tuple[SkillDescriptor, ...]:
        nonlocal scans
        scans += 1
        return (descriptor,)

    try:
        thread, _ = store.create_thread("Idempotent")
        service = TurnService(
            store,
            cast(AgentScheduler, scheduler),
            ConfigStore(tmp_path),
            lambda _value: None,
            load_identity_core(),
            snapshot,
        )
        params = {
            "threadId": thread.id,
            "branchId": thread.default_branch_id,
            "content": "Use demo",
            "providerId": "scripted",
            "modelId": "scripted-v1",
            "clientRequestId": "request-skills",
        }

        first = service.start_turn(params)
        repeated = service.start_turn(params)

        assert repeated.result == first.result
        assert scans == 1
        assert len(scheduler.reserved) == 1
        assert store.get_run(str(first.result["runId"])).skills == (descriptor,)
    finally:
        store.close()


async def _discard_event(_event: JournalEvent) -> None:
    return None


@pytest.mark.asyncio
async def test_runtime_router_exposes_skill_list_and_toggle(tmp_path: Path) -> None:
    _write_skill(tmp_path / "skills", "demo", "Router Skill")
    store = SqliteRuntimeStore(tmp_path / "state.db")
    application = RuntimeApplication(
        store,
        _discard_event,
        memory_store=SqliteMemoryStore(tmp_path / "memory.db"),
    )
    try:
        listed = await application.router.dispatch(1, "skill.list", {})
        listed_result = cast(dict[str, object], listed.response["result"])
        skills = cast(list[dict[str, object]], listed_result["skills"])
        assert [(skill["name"], skill["enabled"]) for skill in skills] == [("demo", True)]

        toggled = await application.router.dispatch(
            2,
            "skill.set_enabled",
            {"name": "demo", "enabled": False},
        )
        toggled_result = cast(dict[str, object], toggled.response["result"])
        skill = cast(dict[str, object], toggled_result["skill"])
        assert skill["enabled"] is False
        assert ConfigDocumentStore(tmp_path).read_section("skills") == {
            "disabled": ["demo"]
        }
    finally:
        await application.close()
        store.close()
