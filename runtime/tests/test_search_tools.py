from __future__ import annotations

from pathlib import Path

import pytest

from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.errors import RunCancelled
from ikaros_runtime.tools.core import (
    ToolCall,
    ToolExecutionContext,
    ToolExecutor,
    ToolRegistry,
    ToolResult,
)
from ikaros_runtime.tools.glob import GlobTool
from ikaros_runtime.tools.grep import GrepTool
from ikaros_runtime.tools.list import ListTool
from ikaros_runtime.tools.policy import FullAccessPolicy
from ikaros_runtime.tools.traversal import format_timestamp


def _context(*, default_cwd: str | None = None) -> ToolExecutionContext:
    return ToolExecutionContext("run_test", 1, "item_test", "thread_test", default_cwd)


def _search_executor() -> ToolExecutor:
    return ToolExecutor(
        ToolRegistry([GlobTool(), GrepTool(), ListTool()]),
        FullAccessPolicy(),
    )


async def _call(
    name: str,
    arguments: dict[str, object],
    *,
    default_cwd: str | None = None,
    cancellation: CancellationToken | None = None,
) -> ToolResult:
    return await _search_executor().execute(
        ToolCall(f"call_{name}", name, arguments),
        cancellation=cancellation or CancellationToken(),
        context=_context(default_cwd=default_cwd),
    )


def _workspace(tmp_path: Path) -> Path:
    (tmp_path / "src").mkdir()
    (tmp_path / "src" / "app.py").write_text(
        "import os\n\n\ndef main():\n    return 1\n",
        encoding="utf-8",
    )
    (tmp_path / "src" / "notes.md").write_text("# Notes\n", encoding="utf-8")
    (tmp_path / "docs").mkdir()
    (tmp_path / "docs" / "guide.md").write_text("See app.py for details\n", encoding="utf-8")
    (tmp_path / "node_modules").mkdir()
    (tmp_path / "node_modules" / "lib.py").write_text("def main():\n    pass\n", encoding="utf-8")
    (tmp_path / ".git").mkdir()
    (tmp_path / ".git" / "hook.py").write_text("def main():\n", encoding="utf-8")
    (tmp_path / "logo.png").write_bytes(b"\x89PNG\r\n\x1a\n\x00\x00\x00")
    return tmp_path


def _glob_paths(output: str) -> list[str]:
    return [line.split("\t")[0] for line in output.splitlines() if "\t" in line]


@pytest.mark.asyncio
async def test_glob_matches_nested_patterns_with_size_and_mtime(tmp_path: Path) -> None:
    workspace = _workspace(tmp_path)
    app = workspace / "src" / "app.py"

    result = await _call("glob", {"pattern": "**/*.py", "path": str(workspace)})

    assert result.ok is True
    assert _glob_paths(result.output) == [str(app)]
    assert f"{app}\t{app.stat().st_size}\t{format_timestamp(app.stat().st_mtime)}" in result.output
    assert "(1 files under" in result.output
    assert result.details["entryCount"] == 1
    assert result.details["pattern"] == "**/*.py"
    assert result.details["path"] == str(workspace.resolve())
    assert result.details["truncated"] is False


@pytest.mark.asyncio
async def test_glob_skips_ignored_directories_and_binary_files(tmp_path: Path) -> None:
    workspace = _workspace(tmp_path)

    result = await _call("glob", {"pattern": "**/*", "path": str(workspace)})

    assert result.ok is True
    paths = _glob_paths(result.output)
    assert str(workspace / "node_modules" / "lib.py") not in paths
    assert str(workspace / ".git" / "hook.py") not in paths
    assert str(workspace / "logo.png") not in paths
    assert str(workspace / "src" / "notes.md") in paths


@pytest.mark.asyncio
async def test_glob_reports_truncation_at_max_results(tmp_path: Path) -> None:
    workspace = tmp_path / "many"
    workspace.mkdir()
    for index in range(5):
        (workspace / f"file{index}.txt").write_text("x", encoding="utf-8")

    result = await _call(
        "glob",
        {"pattern": "*.txt", "path": str(workspace), "maxResults": 2},
    )

    assert result.ok is True
    assert result.details["entryCount"] == 2
    assert result.details["truncated"] is True
    assert "more matches exist" in result.output


@pytest.mark.asyncio
async def test_glob_matches_bare_patterns_against_file_names(tmp_path: Path) -> None:
    workspace = _workspace(tmp_path)

    result = await _call("glob", {"pattern": "*.py", "path": str(workspace)})

    assert result.ok is True
    assert str(workspace / "src" / "app.py") in result.output


@pytest.mark.asyncio
async def test_glob_rejects_invalid_arguments(tmp_path: Path) -> None:
    workspace = _workspace(tmp_path)

    unknown = await _call(
        "glob",
        {"pattern": "*.py", "path": str(workspace), "extra": True},
    )
    assert unknown.ok is False
    assert unknown.details["errorCode"] == "invalid_arguments"

    absolute = await _call("glob", {"pattern": "/etc/*.conf", "path": str(workspace)})
    assert absolute.ok is False
    assert absolute.details["errorCode"] == "invalid_pattern"

    oversized = await _call(
        "glob",
        {"pattern": "*.py", "path": str(workspace), "maxResults": 0},
    )
    assert oversized.ok is False
    assert oversized.details["errorCode"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_glob_rejects_paths_that_are_not_directories(tmp_path: Path) -> None:
    workspace = _workspace(tmp_path)

    result = await _call(
        "glob",
        {"pattern": "*.py", "path": str(workspace / "src" / "app.py")},
    )

    assert result.ok is False
    assert result.details["errorCode"] == "path_not_directory"


@pytest.mark.asyncio
async def test_grep_returns_matches_with_optional_context(tmp_path: Path) -> None:
    workspace = _workspace(tmp_path)
    app = workspace / "src" / "app.py"

    result = await _call(
        "grep",
        {
            "pattern": "def main",
            "path": str(workspace),
            "include": "*.py",
            "contextLines": 1,
        },
    )

    assert result.ok is True
    assert f"{app}:4:def main():" in result.output
    assert f"{app}-3-" in result.output
    assert f"{app}-5-    return 1" in result.output
    assert result.details["matchCount"] == 1
    assert result.details["scannedFiles"] == 1
    assert result.details["truncated"] is False


@pytest.mark.asyncio
async def test_grep_honors_ignore_case_and_include_paths(tmp_path: Path) -> None:
    workspace = _workspace(tmp_path)

    result = await _call(
        "grep",
        {
            "pattern": "see APP.PY",
            "path": str(workspace),
            "include": "docs/*.md",
            "ignoreCase": True,
        },
    )

    assert result.ok is True
    assert f"{workspace / 'docs' / 'guide.md'}:1:See app.py for details" in result.output
    assert result.details["matchCount"] == 1


@pytest.mark.asyncio
async def test_grep_truncates_long_lines_and_respects_max_matches(tmp_path: Path) -> None:
    workspace = tmp_path / "long"
    workspace.mkdir()
    long_line = "needle " + "x" * 4_000
    (workspace / "a.txt").write_text(
        "\n".join(["needle one", "needle two", long_line]),
        encoding="utf-8",
    )

    limited = await _call(
        "grep",
        {"pattern": "needle", "path": str(workspace), "maxMatches": 2},
    )

    assert limited.ok is True
    assert limited.details["matchCount"] == 2
    assert limited.details["truncated"] is True
    assert "match limit 2 reached" in limited.output

    complete = await _call("grep", {"pattern": "needle", "path": str(workspace)})
    assert complete.ok is True
    assert complete.details["matchCount"] == 3
    assert complete.details["lineTruncations"] == 1
    assert "... (line truncated to 2000 chars)" in complete.output


@pytest.mark.asyncio
async def test_grep_skips_binary_files_and_invalid_patterns(tmp_path: Path) -> None:
    workspace = _workspace(tmp_path)
    (workspace / "data.bin").write_bytes(b"needle\x00\x01\x02")

    result = await _call("grep", {"pattern": "needle", "path": str(workspace)})

    assert result.ok is True
    assert "data.bin" not in result.output

    invalid = await _call("grep", {"pattern": "needle(", "path": str(workspace)})
    assert invalid.ok is False
    assert invalid.details["errorCode"] == "invalid_pattern"

    bad_context = await _call(
        "grep",
        {"pattern": "needle", "path": str(workspace), "contextLines": 201},
    )
    assert bad_context.ok is False
    assert bad_context.details["errorCode"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_grep_resolves_relative_paths_from_the_thread_workspace(tmp_path: Path) -> None:
    workspace = _workspace(tmp_path)

    result = await _call(
        "grep",
        {"pattern": "import os", "path": "src"},
        default_cwd=str(workspace),
    )

    assert result.ok is True
    assert result.details["path"] == str((workspace / "src").resolve())
    assert result.details["matchCount"] == 1


@pytest.mark.asyncio
async def test_list_returns_one_directory_level_with_metadata(tmp_path: Path) -> None:
    workspace = _workspace(tmp_path)

    result = await _call("list", {"path": str(workspace)})

    assert result.ok is True
    lines = [line for line in result.output.splitlines() if "\t" in line]
    assert [line.split("\t")[0] for line in lines] == [
        ".git",
        "docs",
        "logo.png",
        "node_modules",
        "src",
    ]
    src_line = next(line for line in lines if line.startswith("src\t"))
    assert src_line.split("\t")[1] == "dir"
    assert src_line.split("\t")[2] == "-"
    logo_line = next(line for line in lines if line.startswith("logo.png\t"))
    assert logo_line.split("\t")[1] == "file"
    assert logo_line.split("\t")[2] == "11"
    assert result.details["entryCount"] == 5
    assert result.details["truncated"] is False
    assert result.details["path"] == str(workspace.resolve())


@pytest.mark.asyncio
async def test_list_defaults_to_the_thread_workspace_and_rejects_files(tmp_path: Path) -> None:
    workspace = _workspace(tmp_path)

    result = await _call("list", {}, default_cwd=str(workspace))

    assert result.ok is True
    assert result.details["path"] == str(workspace.resolve())

    rejected = await _call("list", {"path": str(workspace / "src" / "app.py")})
    assert rejected.ok is False
    assert rejected.details["errorCode"] == "path_not_directory"

    unknown = await _call("list", {"extra": True})
    assert unknown.ok is False
    assert unknown.details["errorCode"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_search_tools_validate_path_lengths_and_cancellation(tmp_path: Path) -> None:
    workspace = _workspace(tmp_path)
    long_path = str(workspace / ("a" * 4_200))

    result = await _call("glob", {"pattern": "*.py", "path": long_path})

    assert result.ok is False
    assert result.details["errorCode"] == "invalid_arguments"

    cancellation = CancellationToken()
    cancellation.cancel()
    with pytest.raises(RunCancelled):
        await _call(
            "grep",
            {"pattern": "needle", "path": str(workspace)},
            cancellation=cancellation,
        )


@pytest.mark.asyncio
async def test_search_tools_do_not_follow_directory_symlinks(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    outside = tmp_path / "outside"
    outside.mkdir()
    (outside / "secret.py").write_text("x = 1\n", encoding="utf-8")
    try:
        (workspace / "link").symlink_to(outside, target_is_directory=True)
    except (OSError, NotImplementedError):
        pytest.skip("directory symlinks are not available on this platform")

    glob_result = await _call("glob", {"pattern": "**/*.py", "path": str(workspace)})
    grep_result = await _call("grep", {"pattern": "x = 1", "path": str(workspace)})
    list_result = await _call("list", {"path": str(workspace)})

    assert glob_result.ok is True
    assert "secret.py" not in glob_result.output
    assert grep_result.ok is True
    assert grep_result.details["matchCount"] == 0
    assert list_result.ok is True
    assert "link\tsymlink" in list_result.output


@pytest.mark.asyncio
async def test_search_tools_descend_arbitrarily_deep_trees(tmp_path: Path) -> None:
    workspace = tmp_path / "deep"
    current = workspace
    for _ in range(40):
        current = current / "d"
    current.mkdir(parents=True)
    buried = current / "buried.py"
    buried.write_text("x = 1\n", encoding="utf-8")

    result = await _call("glob", {"pattern": "**/*.py", "path": str(workspace)})

    assert result.ok is True
    assert _glob_paths(result.output) == [str(buried)]
