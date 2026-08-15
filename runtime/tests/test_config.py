from __future__ import annotations

import os
from pathlib import Path

import pytest
import yaml

from ikaros_runtime.providers.registry import (
    DEEPSEEK_BASE_URL,
    ConfigError,
    ConfigStore,
    ModelInput,
)


def model(model_id: str = "deepseek-chat", name: str = "DeepSeek Chat") -> ModelInput:
    return ModelInput(model_id, name)


def many_models(prefix: str, count: int) -> list[ModelInput]:
    return [
        ModelInput(
            f"{prefix}-{index}-{'x' * 32}",
            f"{prefix} Model {index} {'y' * 48}",
        )
        for index in range(count)
    ]


def test_missing_config_is_empty_without_creating_files(tmp_path: Path) -> None:
    home = tmp_path / ".ikaros"

    store = ConfigStore(home)

    assert not home.exists()
    assert [summary.to_wire() for summary in store.provider_summaries()] == [
        {
            "id": "deepseek",
            "displayName": "DeepSeek",
            "origin": "builtin",
            "configured": False,
            "credentialConfigured": False,
            "health": "unknown",
        }
    ]
    assert store.model_summaries() == ()


def test_deepseek_write_is_explicit_atomic_and_redacted(tmp_path: Path) -> None:
    home = tmp_path / ".ikaros"
    store = ConfigStore(home)
    secret = "sk-this-must-stay-private"

    summary = store.configure_deepseek(api_key=secret, models=[model()])

    document = yaml.safe_load((home / "config.yaml").read_text(encoding="utf-8"))
    assert document == {
        "version": 1,
        "providers": {
            "deepseek": {
                "type": "openai_compatible",
                "preset": "deepseek",
                "base_url": DEEPSEEK_BASE_URL,
                "api_key": secret,
                "models": {
                    "deepseek-chat": {
                        "display_name": "DeepSeek Chat",
                        "enabled": True,
                        "supports_tools": True,
                    }
                },
            }
        },
    }
    assert secret not in repr(store.get_provider("deepseek"))
    assert secret not in repr(summary)
    assert secret not in repr(store.provider_summaries())
    assert not list(home.glob(".config-*.tmp"))
    if os.name != "nt":
        assert (home / "config.yaml").stat().st_mode & 0o777 == 0o600


def test_deepseek_disconnect_removes_models_and_is_idempotent(tmp_path: Path) -> None:
    store = ConfigStore(tmp_path)
    store.configure_deepseek(api_key="sk-private", models=[model()])

    summary = store.disconnect_deepseek()
    repeated = store.disconnect_deepseek()

    assert summary.configured is False
    assert summary.credential_configured is False
    assert repeated == summary
    assert store.get_provider("deepseek") is None
    assert store.model_summaries() == ()
    assert not store.path.exists()


def test_deepseek_disconnect_preserves_custom_provider_across_reload(tmp_path: Path) -> None:
    custom_key = "sk-custom-must-survive"
    store = ConfigStore(tmp_path)
    store.configure_deepseek(api_key="sk-deepseek-remove", models=[model()])
    store.configure_custom(
        provider_id="local",
        display_name="Local Provider",
        base_url="http://127.0.0.1:8080/v1",
        api_key=custom_key,
        headers=None,
        models=[model("local-model", "Local Model")],
    )
    store.set_model_enabled("local", "local-model", False)

    store.disconnect_deepseek()

    assert store.get_provider("deepseek") is None
    custom = store.get_provider("local")
    assert custom is not None
    assert custom.api_key == custom_key
    assert [(item.id, item.display_name, item.enabled) for item in custom.models] == [
        ("local-model", "Local Model", False)
    ]

    reloaded = ConfigStore(tmp_path)
    assert reloaded.get_provider("deepseek") is None
    restored_custom = reloaded.get_provider("local")
    assert restored_custom is not None
    assert restored_custom.api_key == custom_key
    assert [(item.id, item.display_name, item.enabled) for item in restored_custom.models] == [
        ("local-model", "Local Model", False)
    ]
    assert reloaded.protected_values() == (custom_key,)
    assert [(item.provider_id, item.id) for item in reloaded.model_summaries()] == [
        ("local", "local-model")
    ]


def test_custom_provider_supports_optional_key_headers_and_reload(tmp_path: Path) -> None:
    store = ConfigStore(tmp_path)
    summary = store.configure_custom(
        provider_id="Local.API",
        display_name="Local API",
        base_url="http://127.0.0.1:8080/v1/",
        api_key=None,
        headers={"X-Tenant": "tenant-secret", "Authorization": "Bearer local-secret"},
        models=[model("local/model", "Local Model")],
    )

    assert summary.credential_configured is True
    assert store.protected_values() == ("tenant-secret", "Bearer local-secret")
    reloaded = ConfigStore(tmp_path)
    provider = reloaded.get_provider("local.api")
    assert provider is not None
    assert provider.base_url == "http://127.0.0.1:8080/v1"
    assert provider.api_key is None
    assert provider.header_map() == {
        "X-Tenant": "tenant-secret",
        "Authorization": "Bearer local-secret",
    }
    assert reloaded.provider_summaries()[1].credential_configured is True
    assert reloaded.protected_values() == ("tenant-secret", "Bearer local-secret")
    assert "tenant-secret" not in repr(provider)


def test_public_execution_fingerprint_ignores_credential_rotation(tmp_path: Path) -> None:
    store = ConfigStore(tmp_path)
    store.configure_custom(
        provider_id="local",
        display_name="Local",
        base_url="http://127.0.0.1:8080/v1",
        api_key="first-api-secret",
        headers={"X-Tenant": "first-header-secret"},
        models=[model("local-model", "Local Model")],
    )
    first = store.execution_snapshot("local", "local-model")

    store.configure_custom(
        provider_id="local",
        display_name="Local",
        base_url="http://127.0.0.1:8080/v1",
        api_key="second-api-secret",
        headers={"X-Tenant": "second-header-secret"},
        models=[model("local-model", "Local Model")],
    )
    second = store.execution_snapshot("local", "local-model")

    assert second == first
    assert second.fingerprint == first.fingerprint
    serialized = repr(second)
    assert "first-api-secret" not in serialized
    assert "first-header-secret" not in serialized
    assert "second-api-secret" not in serialized
    assert "second-header-secret" not in serialized


def test_credentials_cannot_overlap_public_provider_or_model_fields(tmp_path: Path) -> None:
    protected = "public-field-secret-sentinel"
    store = ConfigStore(tmp_path)

    with pytest.raises(ConfigError, match="public fields") as own_collision:
        store.configure_custom(
            provider_id="local",
            display_name="Local",
            base_url="http://127.0.0.1:8080/v1",
            api_key=None,
            headers={"X-Protected": protected},
            models=[model("local-model", protected)],
        )

    assert protected not in str(own_collision.value)
    assert not store.path.exists()

    with pytest.raises(ConfigError, match="public fields"):
        store.configure_custom(
            provider_id="local",
            display_name="Local",
            base_url="http://127.0.0.1:8080/v1",
            api_key="DeepSeek",
            headers=None,
            models=[model("local-model", "Local Model")],
        )

    store.configure_deepseek(api_key=protected, models=[model()])
    with pytest.raises(ConfigError, match="public fields") as cross_provider_collision:
        store.configure_custom(
            provider_id="local",
            display_name=protected,
            base_url="http://127.0.0.1:8080/v1",
            api_key=None,
            headers=None,
            models=[model("local-model", "Local Model")],
        )

    assert protected not in str(cross_provider_collision.value)
    assert [summary.id for summary in store.provider_summaries()] == ["deepseek"]


def test_rotated_credentials_cannot_be_promoted_into_public_fields(tmp_path: Path) -> None:
    old_secret = "sk-old-rotation-sentinel"
    new_secret = "sk-new-rotation-sentinel"
    store = ConfigStore(tmp_path)
    store.configure_custom(
        provider_id="local",
        display_name="Local",
        base_url="http://127.0.0.1:9/v1",
        api_key=old_secret,
        headers=None,
        models=[model("local-model", "Local Model")],
    )
    original_file = store.path.read_bytes()

    with pytest.raises(ConfigError, match="public fields") as captured:
        store.configure_custom(
            provider_id="local",
            display_name=old_secret,
            base_url="http://127.0.0.1:9/v1",
            api_key=new_secret,
            headers=None,
            models=[model("local-model", "Local Model")],
        )

    with pytest.raises(ConfigError, match="public fields"):
        store.configure_custom(
            provider_id="local",
            display_name="Local",
            base_url=f"http://127.0.0.1:9/v1/{old_secret}",
            api_key=new_secret,
            headers=None,
            models=[model("local-model", "Local Model")],
        )

    assert old_secret not in str(captured.value)
    assert new_secret not in str(captured.value)
    assert store.path.read_bytes() == original_file
    provider = store.get_provider("local")
    assert provider is not None
    assert provider.display_name == "Local"
    assert store.protected_values() == (old_secret,)
    reloaded = ConfigStore(tmp_path)
    assert reloaded.get_provider("local") is not None
    assert reloaded.get_provider("local").display_name == "Local"  # type: ignore[union-attr]
    assert reloaded.protected_values() == (old_secret,)


def test_remove_last_custom_provider_removes_empty_config(tmp_path: Path) -> None:
    store = ConfigStore(tmp_path)
    store.configure_custom(
        provider_id="local",
        display_name="Local",
        base_url="http://localhost:8000/v1",
        api_key=None,
        headers=None,
        models=[model("model", "Model")],
    )

    store.remove_custom("local")

    assert not store.path.exists()
    assert store.provider_summaries()[0].configured is False


def test_model_enablement_is_explicit_and_resolution_requires_runnable_model(
    tmp_path: Path,
) -> None:
    store = ConfigStore(tmp_path)
    store.configure_deepseek(api_key="sk-private", models=[model()])

    disabled = store.set_model_enabled("deepseek", "deepseek-chat", False)

    assert disabled.enabled is False
    with pytest.raises(ConfigError, match="disabled"):
        store.resolve_model("deepseek", "deepseek-chat")
    enabled = store.set_model_enabled("deepseek", "deepseek-chat", True)
    provider, configured_model = store.resolve_model("deepseek", "deepseek-chat")
    assert enabled.enabled is True
    assert provider.id == "deepseek"
    assert configured_model.id == "deepseek-chat"


@pytest.mark.parametrize(
    "base_url",
    [
        "https://example.com／secret-url-sentinel/v1",
        "https://example.com:secret-url-sentinel/v1",
        "https://example.com:99999/secret-url-sentinel",
    ],
)
def test_malformed_base_url_errors_do_not_retain_the_rejected_value(
    tmp_path: Path,
    base_url: str,
) -> None:
    store = ConfigStore(tmp_path)

    with pytest.raises(ConfigError) as captured:
        store.configure_custom(
            provider_id="custom",
            display_name="Custom",
            base_url=base_url,
            api_key=None,
            headers=None,
            models=[model("model", "Model")],
        )

    error = captured.value
    assert "secret-url-sentinel" not in str(error)
    assert "secret-url-sentinel" not in repr(error)
    assert error.__cause__ is None
    assert error.__context__ is None
    assert not store.path.exists()


@pytest.mark.parametrize("secret", ["sk-tab\tsecret", "sk-control\x7fsecret"])
def test_credentials_reject_all_control_characters(tmp_path: Path, secret: str) -> None:
    store = ConfigStore(tmp_path)

    with pytest.raises(ConfigError, match="invalid format"):
        store.configure_deepseek(api_key=secret, models=[model()])

    assert not store.path.exists()


def test_reconfiguring_a_provider_preserves_existing_model_enablement(tmp_path: Path) -> None:
    store = ConfigStore(tmp_path)
    store.configure_deepseek(api_key="sk-first", models=[model(), model("new", "New")])
    store.set_model_enabled("deepseek", "deepseek-chat", False)

    store.configure_deepseek(
        api_key="sk-second",
        models=[model("deepseek-chat", "Renamed"), model("added", "Added")],
    )

    summaries = {summary.id: summary for summary in store.model_summaries()}
    assert summaries["deepseek-chat"].display_name == "Renamed"
    assert summaries["deepseek-chat"].enabled is False
    assert summaries["added"].enabled is True
    assert "new" not in summaries


@pytest.mark.parametrize(
    ("provider_id", "base_url", "api_key", "headers", "message"),
    [
        ("deepseek", "https://example.com/v1", None, None, "non-reserved"),
        ("custom", "file:///tmp/model", None, None, "HTTP"),
        ("custom", "https://user:pass@example.com/v1", None, None, "HTTP"),
        ("custom", "https://example.com/v1?secret=yes", None, None, "HTTP"),
        ("custom", "https://example.com/v1", "sk-key", {"authorization": "x"}, "header"),
        ("custom", "https://example.com/v1", None, {"X-Test": "a\r\nb"}, "header"),
        ("custom", "https://example.com/v1", None, {"Host": "evil"}, "header"),
        (
            "custom",
            "https://example.com/v1",
            None,
            {"X-Tenant": "one", "x-tenant": "two"},
            "header",
        ),
    ],
)
def test_custom_provider_rejects_ambiguous_or_unsafe_transport_configuration(
    tmp_path: Path,
    provider_id: str,
    base_url: str,
    api_key: str | None,
    headers: dict[str, str] | None,
    message: str,
) -> None:
    store = ConfigStore(tmp_path)

    with pytest.raises(ConfigError, match=message):
        store.configure_custom(
            provider_id=provider_id,
            display_name="Custom",
            base_url=base_url,
            api_key=api_key,
            headers=headers,
            models=[model()],
        )

    assert not store.path.exists()


def test_model_records_are_required_unique_and_not_synthesized(tmp_path: Path) -> None:
    store = ConfigStore(tmp_path)

    with pytest.raises(ConfigError, match="at least one"):
        store.configure_deepseek(api_key="sk-private", models=[])
    with pytest.raises(ConfigError, match="unique"):
        store.configure_deepseek(
            api_key="sk-private",
            models=[model("same", "One"), model("same", "Two")],
        )

    assert store.model_summaries() == ()
    assert not store.path.exists()


def test_parse_failures_never_include_yaml_source_or_exception_cause(tmp_path: Path) -> None:
    secret = "sk-parse-leak-sentinel"
    path = tmp_path / "config.yaml"
    path.write_text(f"version: [\napi_key: {secret}\n", encoding="utf-8")

    with pytest.raises(ConfigError) as captured:
        ConfigStore(tmp_path)

    assert secret not in str(captured.value)
    assert secret not in repr(captured.value)
    assert captured.value.__cause__ is None
    assert captured.value.__context__ is None


def test_invalid_loaded_secret_and_header_values_are_not_echoed(tmp_path: Path) -> None:
    secret = "sk-loaded-leak-sentinel"
    path = tmp_path / "config.yaml"
    path.write_text(
        "\n".join(
            [
                "version: 1",
                "providers:",
                "  custom:",
                "    type: openai_compatible",
                "    display_name: Custom",
                "    base_url: https://example.com/v1",
                f'    api_key: "{secret}\\nleak"',
                "    models:",
                "      model:",
                "        display_name: Model",
                "        enabled: true",
                "        supports_tools: true",
            ]
        ),
        encoding="utf-8",
    )

    with pytest.raises(ConfigError) as captured:
        ConfigStore(tmp_path)

    assert secret not in str(captured.value)
    assert captured.value.__cause__ is None


def test_loaded_document_is_strict_and_never_creates_defaults(tmp_path: Path) -> None:
    (tmp_path / "config.yaml").write_text(
        "version: 1\nproviders: {}\ndefault:\n  provider: deepseek\n",
        encoding="utf-8",
    )

    with pytest.raises(ConfigError, match="top-level"):
        ConfigStore(tmp_path)


def test_duplicate_yaml_keys_are_rejected_instead_of_silently_overwritten(
    tmp_path: Path,
) -> None:
    secret = "sk-duplicate-leak-sentinel"
    (tmp_path / "config.yaml").write_text(
        "\n".join(
            [
                "version: 1",
                "providers:",
                "  deepseek:",
                "    type: openai_compatible",
                "    preset: deepseek",
                f"    base_url: {DEEPSEEK_BASE_URL}",
                f"    api_key: {secret}",
                "    api_key: overwritten",
                "    models:",
                "      model:",
                "        display_name: Model",
                "        enabled: true",
                "        supports_tools: true",
            ]
        ),
        encoding="utf-8",
    )

    with pytest.raises(ConfigError) as captured:
        ConfigStore(tmp_path)

    assert secret not in str(captured.value)
    assert captured.value.__context__ is None


def test_loaded_provider_and_model_ids_must_be_unique_after_normalization(
    tmp_path: Path,
) -> None:
    def custom_provider(display_name: str, models: dict[str, object]) -> dict[str, object]:
        return {
            "type": "openai_compatible",
            "display_name": display_name,
            "base_url": "https://example.com/v1",
            "models": models,
        }

    model_row = {
        "display_name": "Model",
        "enabled": True,
        "supports_tools": True,
    }
    path = tmp_path / "config.yaml"
    path.write_text(
        yaml.safe_dump(
            {
                "version": 1,
                "providers": {
                    "Foo": custom_provider("First", {"model": model_row}),
                    "foo": custom_provider("Second", {"model": model_row}),
                },
            },
            sort_keys=False,
        ),
        encoding="utf-8",
    )

    with pytest.raises(ConfigError, match="provider IDs.*unique"):
        ConfigStore(tmp_path)

    path.write_text(
        yaml.safe_dump(
            {
                "version": 1,
                "providers": {
                    "foo": custom_provider(
                        "Provider",
                        {"x": model_row, " x ": {**model_row, "enabled": False}},
                    )
                },
            },
            sort_keys=False,
        ),
        encoding="utf-8",
    )

    with pytest.raises(ConfigError, match="model IDs.*unique"):
        ConfigStore(tmp_path)


def test_failed_atomic_replace_preserves_file_memory_and_cleans_temporary_file(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    store = ConfigStore(tmp_path)
    store.configure_deepseek(api_key="sk-original", models=[model("original", "Original")])
    original_file = store.path.read_bytes()

    def fail_replace(
        source: str | bytes | os.PathLike[str] | os.PathLike[bytes], target: object
    ) -> None:
        del source, target
        raise OSError("simulated replace failure")

    monkeypatch.setattr("ikaros_runtime.config.os.replace", fail_replace)

    with pytest.raises(ConfigError) as captured:
        store.configure_deepseek(api_key="sk-new-secret", models=[model("new", "New")])

    assert "sk-new-secret" not in str(captured.value)
    assert store.path.read_bytes() == original_file
    assert store.get_provider("deepseek") is not None
    assert store.get_provider("deepseek").models[0].id == "original"  # type: ignore[union-attr]
    assert not list(tmp_path.glob(".config-*.tmp"))


def test_successful_replace_does_not_depend_on_post_commit_target_chmod(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    store = ConfigStore(tmp_path)
    store.configure_deepseek(api_key="sk-original", models=[model("original", "Original")])
    original_chmod = os.chmod
    target_chmod_attempted = False

    def reject_target_chmod(
        path: str | bytes | os.PathLike[str] | os.PathLike[bytes],
        mode: int,
    ) -> None:
        nonlocal target_chmod_attempted
        if Path(os.fsdecode(path)) == store.path:
            target_chmod_attempted = True
            raise OSError("target chmod must not run after atomic replace")
        original_chmod(path, mode)

    monkeypatch.setattr("ikaros_runtime.config.os.chmod", reject_target_chmod)

    store.configure_deepseek(api_key="sk-replaced", models=[model("new", "New")])

    assert target_chmod_attempted is False
    assert store.get_provider("deepseek") is not None
    assert store.get_provider("deepseek").models[0].id == "new"  # type: ignore[union-attr]
    reloaded = ConfigStore(tmp_path)
    assert reloaded.get_provider("deepseek") is not None
    assert reloaded.get_provider("deepseek").models[0].id == "new"  # type: ignore[union-attr]
    assert reloaded.protected_values() == ("sk-replaced",)
    assert not list(tmp_path.glob(".config-*.tmp"))


def test_large_config_and_environment_credentials_are_never_implicit(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("DEEPSEEK_API_KEY", "sk-environment-must-be-ignored")
    empty_home = tmp_path / "empty"
    store = ConfigStore(empty_home)
    assert store.provider_summaries()[0].configured is False
    assert not empty_home.exists()

    oversized_home = tmp_path / "oversized"
    oversized_home.mkdir()
    (oversized_home / "config.yaml").write_bytes(b"x" * (256 * 1024 + 1))
    with pytest.raises(ConfigError, match="size"):
        ConfigStore(oversized_home)


def test_single_oversized_configuration_is_rejected_before_disk_or_memory_change(
    tmp_path: Path,
) -> None:
    store = ConfigStore(tmp_path)
    secret = "sk-oversized-write-sentinel"

    with pytest.raises(ConfigError, match="size") as captured:
        store.configure_deepseek(
            api_key=secret,
            models=many_models("oversized", 1800),
        )

    assert secret not in str(captured.value)
    assert store.get_provider("deepseek") is None
    assert not store.path.exists()
    assert not list(tmp_path.glob(".config-*.tmp"))


def test_cumulative_size_failure_preserves_existing_disk_and_memory(
    tmp_path: Path,
) -> None:
    store = ConfigStore(tmp_path)
    store.configure_deepseek(
        api_key="sk-existing-size-sentinel",
        models=many_models("existing", 900),
    )
    original_file = store.path.read_bytes()

    with pytest.raises(ConfigError, match="size"):
        store.configure_custom(
            provider_id="additional",
            display_name="Additional",
            base_url="http://127.0.0.1:9/v1",
            api_key=None,
            headers=None,
            models=many_models("additional", 900),
        )

    assert store.path.read_bytes() == original_file
    assert store.get_provider("additional") is None
    assert store.get_provider("deepseek") is not None
    assert len(store.get_provider("deepseek").models) == 900  # type: ignore[union-attr]
    reloaded = ConfigStore(tmp_path)
    assert reloaded.get_provider("additional") is None
    assert reloaded.get_provider("deepseek") is not None
    assert len(reloaded.get_provider("deepseek").models) == 900  # type: ignore[union-attr]
    assert not list(tmp_path.glob(".config-*.tmp"))
