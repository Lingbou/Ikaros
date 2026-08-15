from __future__ import annotations

import os
from pathlib import Path
from typing import cast

import pytest
import yaml

from ikaros_runtime.config import ConfigDocumentStore
from ikaros_runtime.errors import ConfigError
from ikaros_runtime.providers.registry import ConfigStore, ModelInput


def test_missing_document_has_no_sections_and_does_not_create_runtime_home(
    tmp_path: Path,
) -> None:
    runtime_home = tmp_path / ".ikaros"

    document = ConfigDocumentStore(runtime_home)

    assert document.path == runtime_home / "config.yaml"
    assert document.read_section("providers") is None
    assert document.read_section("skills") is None
    assert not runtime_home.exists()


def test_section_snapshots_are_isolated_and_replacement_preserves_other_sections(
    tmp_path: Path,
) -> None:
    document = ConfigDocumentStore(tmp_path)
    document.replace_section("skills", {"disabled": ["imagegen"]})

    snapshot = document.read_section("skills")
    assert snapshot is not None
    cast(list[object], snapshot["disabled"]).append("mutated-by-caller")
    assert document.read_section("skills") == {"disabled": ["imagegen"]}

    document.replace_section("providers", {"example": {"opaque": "provider-value"}})

    assert document.read_section("skills") == {"disabled": ["imagegen"]}
    assert document.read_section("providers") == {
        "example": {"opaque": "provider-value"}
    }


def test_provider_facade_preserves_skills_credentials_headers_and_models(
    tmp_path: Path,
) -> None:
    document = ConfigDocumentStore(tmp_path)
    document.replace_section("skills", {"disabled": ["disabled-skill"]})
    providers = ConfigStore(document)
    providers.configure_custom(
        provider_id="local",
        display_name="Local",
        base_url="https://example.com/v1",
        api_key="sk-provider-secret",
        headers={"X-Tenant": "tenant-secret"},
        models=[ModelInput("model-one", "Model One"), ModelInput("model-two", "Model Two")],
    )
    providers.set_model_enabled("local", "model-two", False)

    reloaded_document = ConfigDocumentStore(tmp_path)
    reloaded_providers = ConfigStore(reloaded_document)
    provider = reloaded_providers.get_provider("local")

    assert reloaded_document.read_section("skills") == {"disabled": ["disabled-skill"]}
    assert provider is not None
    assert provider.api_key == "sk-provider-secret"
    assert provider.header_map() == {"X-Tenant": "tenant-secret"}
    assert [(item.id, item.enabled) for item in provider.models] == [
        ("model-one", True),
        ("model-two", False),
    ]


def test_removing_last_provider_keeps_non_empty_skills_until_skills_are_removed(
    tmp_path: Path,
) -> None:
    document = ConfigDocumentStore(tmp_path)
    document.replace_section("skills", {"disabled": ["disabled-skill"]})
    providers = ConfigStore(document)
    providers.configure_custom(
        provider_id="local",
        display_name="Local",
        base_url="https://example.com/v1",
        api_key=None,
        headers=None,
        models=[ModelInput("model", "Model")],
    )

    providers.remove_custom("local")

    assert document.path.exists()
    assert document.read_section("providers") is None
    assert document.read_section("skills") == {"disabled": ["disabled-skill"]}
    assert yaml.safe_load(document.path.read_text(encoding="utf-8")) == {
        "version": 1,
        "skills": {"disabled": ["disabled-skill"]},
    }

    document.remove_section("skills")
    assert not document.path.exists()


@pytest.mark.parametrize(
    "source",
    [
        "version: 1\ndefault: {}\n",
        "version: 1\nskills: []\n",
        "version: true\nproviders: {}\n",
    ],
)
def test_document_rejects_unknown_sections_invalid_section_shapes_and_boolean_version(
    tmp_path: Path,
    source: str,
) -> None:
    (tmp_path / "config.yaml").write_text(source, encoding="utf-8")

    with pytest.raises(ConfigError):
        ConfigDocumentStore(tmp_path)


def test_document_rejects_duplicate_keys_and_invalid_utf8_without_leaking_source(
    tmp_path: Path,
) -> None:
    secret = "document-secret-sentinel"
    path = tmp_path / "config.yaml"
    path.write_text(
        f"version: 1\nskills:\n  disabled: [{secret}]\n  disabled: []\n",
        encoding="utf-8",
    )

    with pytest.raises(ConfigError) as duplicate:
        ConfigDocumentStore(tmp_path)

    assert secret not in str(duplicate.value)
    assert duplicate.value.__cause__ is None
    assert duplicate.value.__context__ is None

    path.write_bytes(b"version: 1\nskills:\n  invalid: \xff\n")
    with pytest.raises(ConfigError) as invalid_utf8:
        ConfigDocumentStore(tmp_path)

    assert invalid_utf8.value.__cause__ is None
    assert invalid_utf8.value.__context__ is None


def test_document_atomic_replace_failure_preserves_all_sections_and_cleans_temp(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    document = ConfigDocumentStore(tmp_path)
    document.replace_section("skills", {"disabled": ["original"]})
    original = document.path.read_bytes()

    def fail_replace(
        source: str | bytes | os.PathLike[str] | os.PathLike[bytes],
        target: str | bytes | os.PathLike[str] | os.PathLike[bytes],
    ) -> None:
        del source, target
        raise OSError("failure containing secret-sentinel")

    monkeypatch.setattr("ikaros_runtime.config.os.replace", fail_replace)

    with pytest.raises(ConfigError) as captured:
        document.replace_section("providers", {"provider": {"api_key": "secret-sentinel"}})

    assert "secret-sentinel" not in str(captured.value)
    assert captured.value.__cause__ is None
    assert captured.value.__context__ is None
    assert document.path.read_bytes() == original
    assert document.read_section("providers") is None
    assert document.read_section("skills") == {"disabled": ["original"]}
    assert not list(tmp_path.glob(".config-*.tmp"))
