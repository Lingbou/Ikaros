<#
.SYNOPSIS
Installs the Ikaros Runtime into <runtime home>/runtime and writes install.json.

.DESCRIPTION
The desktop app discovers a separately installed Runtime through
<runtime home>/runtime/install.json (see src/main/runtimeProvision.ts). This
script creates an isolated virtual environment under <runtime home>/runtime/<version>,
installs the Runtime with uv, and records the result.

Nothing outside the Runtime home is modified and the system Python is untouched.

.PARAMETER Source
Repository root that contains runtime/. Used when installing from a checkout.

.PARAMETER BundleDir
Directory that ships a built wheel plus runtime-bundle.json. Used by packaged apps.

.PARAMETER Python
Interpreter version uv should provision; the Runtime requires >=3.12,<3.14.

.PARAMETER Check
Only verify an existing installation; install nothing.
#>
[CmdletBinding()]
param(
  [string]$Source = "",
  [string]$BundleDir = "",
  [string]$Python = "3.13",
  [switch]$Check
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-IkarosHome {
  $configured = [Environment]::GetEnvironmentVariable("IKAROS_HOME")
  if ($configured) {
    $trimmed = $configured.Trim()
    if ($trimmed.Length -gt 0) { return [System.IO.Path]::GetFullPath($trimmed) }
  }
  return [System.IO.Path]::GetFullPath(
    (Join-Path ([Environment]::GetFolderPath("UserProfile")) ".ikaros")
  )
}

function Resolve-SourceRoot {
  param([string]$Value)
  if ($Value -and $Value.Trim().Length -gt 0) {
    return (Resolve-Path -LiteralPath $Value.Trim()).Path
  }
  return (Resolve-Path -LiteralPath (Split-Path -Parent $PSScriptRoot)).Path
}

function Read-RuntimeVersion {
  param([string]$RepositoryRoot)
  $pyproject = Join-Path $RepositoryRoot "runtime/pyproject.toml"
  if (-not (Test-Path -LiteralPath $pyproject)) {
    throw "Missing $pyproject; point -Source at the Ikaros repository root."
  }
  $match = Select-String -LiteralPath $pyproject -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1
  if (-not $match) { throw "Could not read the Runtime version from $pyproject." }
  return $match.Matches[0].Groups[1].Value
}

function Read-ProtocolVersion {
  param([string]$RepositoryRoot)
  $manifest = Join-Path $RepositoryRoot "runtime/protocol/runtime-protocol.json"
  if (-not (Test-Path -LiteralPath $manifest)) {
    throw "Missing $manifest; run the protocol generator first."
  }
  $parsed = Get-Content -LiteralPath $manifest -Raw | ConvertFrom-Json
  $value = $parsed.protocolVersion
  # ConvertFrom-Json yields Int32 or Int64 depending on the value; accept both.
  if (-not (($value -is [int]) -or ($value -is [long])) -or [long]$value -lt 1) {
    throw "$manifest has no valid protocolVersion."
  }
  return [int]$value
}

function Read-Bundle {
  param([string]$Directory)
  $manifest = Join-Path $Directory "runtime-bundle.json"
  if (-not (Test-Path -LiteralPath $manifest)) {
    throw "Missing $manifest; the packaged Runtime bundle is incomplete."
  }
  $parsed = Get-Content -LiteralPath $manifest -Raw | ConvertFrom-Json
  $wheel = Join-Path $Directory ([string]$parsed.wheel)
  if (-not (Test-Path -LiteralPath $wheel)) {
    throw "Missing Runtime wheel $wheel."
  }
  return [pscustomobject]@{
    Version = [string]$parsed.version
    ProtocolVersion = [int]$parsed.protocolVersion
    InstallTarget = $wheel
  }
}

function Read-Source {
  param([string]$RepositoryRoot)
  return [pscustomobject]@{
    Version = (Read-RuntimeVersion $RepositoryRoot)
    ProtocolVersion = (Read-ProtocolVersion $RepositoryRoot)
    InstallTarget = (Join-Path $RepositoryRoot "runtime")
  }
}

$runtimeHome = Get-IkarosHome
$runtimeDirectory = Join-Path $runtimeHome "runtime"
$manifestPath = Join-Path $runtimeDirectory "install.json"

function Test-Installation {
  $manifest = Read-Manifest
  if ($null -eq $manifest) {
    Write-Host "Runtime is not installed: $manifestPath"
    return 1
  }
  foreach ($field in @(
    "schemaVersion",
    "runtimeVersion",
    "protocolVersion",
    "runtimeRoot",
    "pythonExecutable",
    "installedAt"
  )) {
    if (-not $manifest.PSObject.Properties[$field]) {
      Write-Host "Manifest is missing '$field': $manifestPath"
      return 1
    }
  }
  if ($manifest.schemaVersion -ne 1) {
    Write-Host "Manifest schema $($manifest.schemaVersion) is not supported."
    return 1
  }
  if (-not (Test-Path -LiteralPath $manifest.pythonExecutable)) {
    Write-Host "Interpreter is missing: $($manifest.pythonExecutable)"
    return 1
  }
  $probe = & $manifest.pythonExecutable -c "import ikaros_runtime" 2>&1
  if ($LASTEXITCODE -ne 0) {
    Write-Host "The installed interpreter cannot import ikaros_runtime: $probe"
    return 1
  }
  Write-Host "Runtime $($manifest.runtimeVersion) (protocol $($manifest.protocolVersion)) is installed."
  Write-Host "  interpreter: $($manifest.pythonExecutable)"
  Write-Host "  manifest:    $manifestPath"
  return 0
}

$utf8 = New-Object System.Text.UTF8Encoding($false)

function Read-Manifest {
  if (-not (Test-Path -LiteralPath $manifestPath)) { return $null }
  return (Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json)
}

if ($Check) { exit (Test-Installation) }

if ($BundleDir -and $BundleDir.Trim().Length -gt 0) {
  $resolved = Read-Bundle (Resolve-Path -LiteralPath $BundleDir.Trim()).Path
} else {
  $resolved = Read-Source (Resolve-SourceRoot $Source)
}

$version = $resolved.Version
$protocolVersion = $resolved.ProtocolVersion
$target = Join-Path $runtimeDirectory $version
$pythonExecutable = Join-Path $target "Scripts/python.exe"

if (-not (Get-Command uv -ErrorAction SilentlyContinue)) {
  throw "uv is required. Install it from https://docs.astral.sh/uv/ and re-run this script."
}

Write-Host "Installing Ikaros Runtime $version (protocol $protocolVersion)"
Write-Host "  package: $($resolved.InstallTarget)"
Write-Host "  target:  $target"

New-Item -ItemType Directory -Force -Path $runtimeDirectory | Out-Null
& uv venv --python $Python --allow-existing $target
if ($LASTEXITCODE -ne 0) { throw "uv venv failed with exit code $LASTEXITCODE." }
& uv pip install --python $pythonExecutable $resolved.InstallTarget
if ($LASTEXITCODE -ne 0) { throw "uv pip install failed with exit code $LASTEXITCODE." }

$manifest = [ordered]@{
  schemaVersion    = 1
  runtimeVersion   = $version
  protocolVersion  = $protocolVersion
  runtimeRoot      = $target
  pythonExecutable = $pythonExecutable
  installedAt      = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ss.fffZ")
}
[System.IO.File]::WriteAllText(
  $manifestPath,
  ($manifest | ConvertTo-Json) + "`n",
  $utf8
)
Write-Host "Wrote $manifestPath"
exit (Test-Installation)
