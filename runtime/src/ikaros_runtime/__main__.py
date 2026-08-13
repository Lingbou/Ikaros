from __future__ import annotations

import argparse
import asyncio
import os
from collections.abc import Sequence
from pathlib import Path

from .bootstrap import ServerSettings, run_runtime_server


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="ikaros-runtime")
    subparsers = parser.add_subparsers(dest="command", required=True)

    serve = subparsers.add_parser("serve", help="start the local Runtime server")
    serve.add_argument("--host", required=True)
    serve.add_argument("--port", required=True, type=int)
    serve.add_argument("--token", required=True)
    serve.add_argument("--parent-pid", required=True, type=int)
    return parser


def main(argv: Sequence[str] | None = None) -> None:
    args = build_parser().parse_args(argv)
    if args.command != "serve":
        raise AssertionError(f"unhandled command: {args.command}")
    settings = ServerSettings(
        host=args.host,
        port=args.port,
        token=args.token,
        parent_pid=args.parent_pid,
        runtime_home=Path(os.environ.get("IKAROS_HOME", Path.home() / ".ikaros")),
    )
    asyncio.run(run_runtime_server(settings))


if __name__ == "__main__":
    main()
