from __future__ import annotations

import argparse
import asyncio
import os
from collections.abc import Sequence
from pathlib import Path

from .bootstrap import ServerSettings, run_runtime_server
from .json_codec import dumps as json_dumps
from .maintenance import (
    backup_runtime_state,
    check_runtime_state,
    repair_runtime_projections,
)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="ikaros-runtime")
    subparsers = parser.add_subparsers(dest="command", required=True)

    serve = subparsers.add_parser("serve", help="start the local Runtime server")
    serve.add_argument("--host", required=True)
    serve.add_argument("--port", required=True, type=int)
    serve.add_argument("--token", required=True)
    serve.add_argument("--parent-pid", required=True, type=int)

    storage = subparsers.add_parser("storage", help="inspect or maintain offline Runtime state")
    storage_commands = storage.add_subparsers(dest="storage_command", required=True)
    storage_commands.add_parser("check", help="validate Journal and query projections")
    backup = storage_commands.add_parser("backup", help="create a verified SQLite backup")
    backup.add_argument("--output", type=Path)
    repair = storage_commands.add_parser(
        "repair-projections",
        help="back up state and atomically rebuild query projections",
    )
    repair.add_argument("--backup-output", type=Path)
    return parser


def main(argv: Sequence[str] | None = None) -> None:
    args = build_parser().parse_args(argv)
    runtime_home = Path(os.environ.get("IKAROS_HOME", Path.home() / ".ikaros"))
    if args.command == "serve":
        settings = ServerSettings(
            host=args.host,
            port=args.port,
            token=args.token,
            parent_pid=args.parent_pid,
            runtime_home=runtime_home,
        )
        asyncio.run(run_runtime_server(settings))
        return
    if args.storage_command == "check":
        check_report = asyncio.run(check_runtime_state(runtime_home))
        result = {"operation": "storage.check", **check_report.to_wire()}
    elif args.storage_command == "backup":
        backup_report = asyncio.run(backup_runtime_state(runtime_home, args.output))
        result = {"operation": "storage.backup", **backup_report.to_wire()}
    elif args.storage_command == "repair-projections":
        repair_report = asyncio.run(
            repair_runtime_projections(runtime_home, args.backup_output)
        )
        result = {
            "operation": "storage.repair-projections",
            **repair_report.to_wire(),
        }
    else:
        raise AssertionError(f"unhandled storage command: {args.storage_command}")
    print(json_dumps(result, ensure_ascii=False, separators=(",", ":")))


if __name__ == "__main__":
    main()
