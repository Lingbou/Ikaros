#!/usr/bin/env bash
# Installs the Ikaros Runtime into <runtime home>/runtime and writes install.json.
#
# The desktop app discovers a separately installed Runtime through
# <runtime home>/runtime/install.json (see src/main/runtimeProvision.ts). This
# script creates an isolated virtual environment under <runtime home>/runtime/<version>,
# installs the Runtime package from a checkout with uv, and records the result.
#
# Nothing outside the Runtime home is modified and the system Python is untouched.
#
# Usage:
#   ./scripts/install-runtime.sh [--source <repo>] [--python 3.13] [--check]
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source_root="$(cd "$script_dir/.." && pwd)"
python_version="3.13"
check_only=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --source)
      source_root="$(cd "$2" && pwd)"
      shift 2
      ;;
    --python)
      python_version="$2"
      shift 2
      ;;
    --check)
      check_only=1
      shift
      ;;
    -h|--help)
      sed -n '2,16p' "${BASH_SOURCE[0]}"
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

ikaros_home="${IKAROS_HOME:-$HOME/.ikaros}"
runtime_dir="$ikaros_home/runtime"
manifest_path="$runtime_dir/install.json"

read_manifest_field() {
  python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$manifest_path" "$1"
}

test_installation() {
  if [[ ! -f "$manifest_path" ]]; then
    echo "Runtime is not installed: $manifest_path"
    return 1
  fi
  local schema python_executable runtime_version protocol_version
  schema="$(read_manifest_field schemaVersion 2>/dev/null || echo "")"
  if [[ "$schema" != "1" ]]; then
    echo "Manifest schema '$schema' is not supported: $manifest_path"
    return 1
  fi
  python_executable="$(read_manifest_field pythonExecutable)"
  runtime_version="$(read_manifest_field runtimeVersion)"
  protocol_version="$(read_manifest_field protocolVersion)"
  if [[ ! -x "$python_executable" ]]; then
    echo "Interpreter is missing: $python_executable"
    return 1
  fi
  if ! "$python_executable" -c 'import ikaros_runtime' >/dev/null 2>&1; then
    echo "The installed interpreter cannot import ikaros_runtime: $python_executable"
    return 1
  fi
  echo "Runtime $runtime_version (protocol $protocol_version) is installed."
  echo "  interpreter: $python_executable"
  echo "  manifest:    $manifest_path"
  return 0
}

if [[ "$check_only" -eq 1 ]]; then
  test_installation
  exit $?
fi

pyproject="$source_root/runtime/pyproject.toml"
protocol_manifest="$source_root/runtime/protocol/runtime-protocol.json"
if [[ ! -f "$pyproject" ]]; then
  echo "Missing $pyproject; point --source at the Ikaros repository root." >&2
  exit 1
fi
if [[ ! -f "$protocol_manifest" ]]; then
  echo "Missing $protocol_manifest; run the protocol generator first." >&2
  exit 1
fi

version="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$pyproject" | head -n 1)"
if [[ -z "$version" ]]; then
  echo "Could not read the Runtime version from $pyproject." >&2
  exit 1
fi
protocol_version="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["protocolVersion"])' "$protocol_manifest")"

target="$runtime_dir/$version"
python_executable="$target/bin/python"

if ! command -v uv >/dev/null 2>&1; then
  echo "uv is required. Install it from https://docs.astral.sh/uv/ and re-run this script." >&2
  exit 1
fi

echo "Installing Ikaros Runtime $version (protocol $protocol_version)"
echo "  source: $source_root/runtime"
echo "  target: $target"

mkdir -p "$runtime_dir"
uv venv --python "$python_version" --allow-existing "$target"
uv pip install --python "$python_executable" "$source_root/runtime"

installed_at="$(date -u +"%Y-%m-%dT%H:%M:%S.000Z")"
cat > "$manifest_path" <<JSON
{
  "schemaVersion": 1,
  "runtimeVersion": "$version",
  "protocolVersion": $protocol_version,
  "runtimeRoot": "$target",
  "pythonExecutable": "$python_executable",
  "installedAt": "$installed_at"
}
JSON

echo "Wrote $manifest_path"
test_installation
