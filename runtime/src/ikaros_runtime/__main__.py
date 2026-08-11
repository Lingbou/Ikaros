from __future__ import annotations

import argparse
import asyncio
from collections.abc import Sequence

from .server import ServerSettings, run_server


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
    )
    asyncio.run(run_server(settings))


if __name__ == "__main__":
    main()
