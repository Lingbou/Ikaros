"""Subprocess test server runner that injects the ScriptedProvider fake."""

from __future__ import annotations

import argparse
import asyncio
import os
from pathlib import Path

from ikaros_runtime.bootstrap import ServerSettings, run_runtime_server

from .scripted_provider import ScriptedProvider, scripted_provider_snapshot


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="test-runtime-server")
    subparsers = parser.add_subparsers(dest="command", required=True)

    serve = subparsers.add_parser("serve", help="start the test Runtime server")
    serve.add_argument("--host", required=True)
    serve.add_argument("--port", required=True, type=int)
    serve.add_argument("--token", required=True)
    serve.add_argument("--parent-pid", required=True, type=int)
    return parser


def main() -> None:
    args = build_parser().parse_args()
    runtime_home = Path(os.environ.get("IKAROS_HOME", Path.home() / ".ikaros"))
    settings = ServerSettings(
        host=args.host,
        port=args.port,
        token=args.token,
        parent_pid=args.parent_pid,
        runtime_home=runtime_home,
    )
    asyncio.run(
        run_runtime_server(
            settings,
            extra_providers={ScriptedProvider.id: ScriptedProvider()},
            provider_snapshot_resolver=scripted_provider_snapshot,
        )
    )


if __name__ == "__main__":
    main()