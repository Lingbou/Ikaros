from __future__ import annotations

from pathlib import Path

import pytest

from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.paths import RuntimePaths
from ikaros_runtime.tools.artifacts import (
    MAX_INLINE_RESULT_BYTES,
    PREVIEW_LINE_COUNT,
    ToolArtifactStore,
    build_preview,
    safe_segment,
)
from ikaros_runtime.tools.core import (
    ToolCall,
    ToolExecutionContext,
    ToolExecutor,
    ToolRegistry,
    ToolResult,
)
from ikaros_runtime.tools.glob import GlobTool
from ikaros_runtime.tools.policy import FullAccessPolicy
from ikaros_runtime.tools.read import ReadTool


def _context() -> ToolExecutionContext:
    return ToolExecutionContext("run_test", 1, "item_test", "thread_test", None)


def _executor(artifacts: ToolArtifactStore | None) -> ToolExecutor:
    return ToolExecutor(
        ToolRegistry([ReadTool(), GlobTool()]),
        FullAccessPolicy(),
        artifacts=artifacts,
    )


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def _oversized_result() -> ToolResult:
    output = "\n".join(f"line {index} {'x' * 40}" for index in range(4_000))
    assert len(output.encode("utf-8")) > MAX_INLINE_RESULT_BYTES
    return ToolResult(
        tool_call_id="call_big",
        tool_name="glob",
        ok=True,
        output=output,
        details={"entryCount": 4_000, "truncated": False},
    )


def test_runtime_paths_own_the_artifacts_directory(tmp_path: Path) -> None:
    paths = RuntimePaths.from_home(tmp_path)

    assert paths.artifacts == tmp_path.resolve() / "artifacts"


@pytest.mark.asyncio
async def test_materialize_spills_oversized_output_with_a_head_preview(tmp_path: Path) -> None:
    store = ToolArtifactStore(tmp_path / "artifacts")
    result = _oversized_result()

    spilled = await store.materialize(result, thread_id="thread_abc", item_id="item_def")

    artifact = tmp_path / "artifacts" / "thread_abc" / "item_def.txt"
    assert spilled.output.startswith("\n".join(result.output.splitlines()[:5]))
    assert f"saved at {artifact}" in spilled.output
    assert f"read offset={PREVIEW_LINE_COUNT + 1}" in spilled.output
    assert spilled.details["artifactPath"] == str(artifact)
    assert spilled.details["artifactBytes"] == len(result.output.encode("utf-8"))
    assert spilled.details["artifactLines"] == 4_000
    assert spilled.details["truncated"] is True
    assert spilled.details["entryCount"] == 4_000
    assert artifact.read_text(encoding="utf-8") == result.output


@pytest.mark.asyncio
async def test_materialize_keeps_small_results_inline(tmp_path: Path) -> None:
    store = ToolArtifactStore(tmp_path / "artifacts")
    result = ToolResult(
        tool_call_id="call_small",
        tool_name="list",
        ok=True,
        output="src\tdir\t-\t2026-10-20T00:00:00Z",
        details={"entryCount": 1, "truncated": False},
    )

    returned = await store.materialize(result, thread_id="thread_abc", item_id="item_def")

    assert returned is result
    assert not (tmp_path / "artifacts").exists()


@pytest.mark.asyncio
async def test_materialize_keeps_the_result_when_the_artifact_cannot_be_written(
    tmp_path: Path,
) -> None:
    blocked = tmp_path / "blocked"
    blocked.write_text("not a directory", encoding="utf-8")
    store = ToolArtifactStore(blocked)
    result = _oversized_result()

    returned = await store.materialize(result, thread_id="thread_abc", item_id="item_def")

    assert returned is result


@pytest.mark.asyncio
async def test_materialize_never_writes_output_that_fails_the_safety_check(
    tmp_path: Path,
) -> None:
    store = ToolArtifactStore(
        tmp_path / "artifacts",
        is_safe=lambda text: "super-secret-value" not in text,
    )
    result = ToolResult(
        tool_call_id="call_secret",
        tool_name="grep",
        ok=True,
        output="super-secret-value\n" + "x" * (MAX_INLINE_RESULT_BYTES + 1),
        details={},
    )

    returned = await store.materialize(result, thread_id="thread_abc", item_id="item_def")

    assert returned is result
    assert not (tmp_path / "artifacts").exists()


@pytest.mark.asyncio
async def test_tool_executor_spills_large_glob_results_and_read_continues(
    tmp_path: Path,
) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    for index in range(1_200):
        (workspace / f"result-file-{index:04d}.txt").write_text("x", encoding="utf-8")
    artifacts = ToolArtifactStore(tmp_path / "artifacts")
    executor = _executor(artifacts)

    result = await executor.execute(
        ToolCall("call_glob", "glob", {"pattern": "*.txt", "path": str(workspace)}),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is True
    artifact = Path(str(result.details["artifactPath"]))
    assert artifact == tmp_path / "artifacts" / "thread_test" / "item_test.txt"
    assert result.details["truncated"] is True
    assert result.details["artifactPreviewLines"] == PREVIEW_LINE_COUNT
    assert _read_text(artifact).count("\n") > 1_000
    assert f"read offset={PREVIEW_LINE_COUNT + 1}" in result.output

    continued = await executor.execute(
        ToolCall(
            "call_read",
            "read",
            {"filePath": str(artifact), "offset": PREVIEW_LINE_COUNT + 1, "limit": 3},
        ),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert continued.ok is True
    assert f"{PREVIEW_LINE_COUNT + 1}: " in continued.output
    assert "artifactPath" not in continued.details


def test_build_preview_bounds_the_head_and_truncates_giant_lines() -> None:
    preview, preview_lines, total_lines = build_preview(
        "\n".join(f"line {index}" for index in range(500))
    )

    assert preview_lines == PREVIEW_LINE_COUNT
    assert total_lines == 500
    assert preview.endswith("line 199")

    giant, giant_lines, giant_total = build_preview("y" * (64 * 1024))

    assert giant_lines == 1
    assert giant_total == 1
    assert giant.endswith("... (preview line truncated)")
    assert len(giant.encode("utf-8")) <= 32 * 1024 + 32


def test_safe_segment_keeps_artifact_paths_inside_the_thread_directory() -> None:
    assert safe_segment("thread_abc-123") == "thread_abc-123"
    assert safe_segment("../../etc/passwd") == "etc_passwd"
    assert safe_segment("..") == "unknown"
    assert safe_segment("") == "unknown"
