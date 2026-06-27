param(
  [string]$Repo = $env:IKAROS_INSTALL_REPO,
  [string]$Version = $env:IKAROS_INSTALL_VERSION,
  [string]$BinDir = $env:IKAROS_INSTALL_BIN_DIR
)

if ([string]::IsNullOrWhiteSpace($Repo)) {
  $Repo = "lingbou/Ikaros"
}
if ([string]::IsNullOrWhiteSpace($Version)) {
  $Version = "latest"
}
if ([string]::IsNullOrWhiteSpace($BinDir)) {
  $BinDir = Join-Path $HOME ".ikaros\bin"
}

$Target = "x86_64-pc-windows-msvc"
if ($Version -eq "latest") {
  $Url = "https://github.com/$Repo/releases/latest/download/ikaros-$Target.zip"
} else {
  $Url = "https://github.com/$Repo/releases/download/$Version/ikaros-$Target.zip"
}

$TempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("ikaros-install-" + [System.Guid]::NewGuid().ToString("N"))
$Archive = Join-Path $TempDir "ikaros.zip"
New-Item -ItemType Directory -Force -Path $TempDir, $BinDir | Out-Null

try {
  Write-Host "install_target: $Target"
  Write-Host "install_url: $Url"
  Invoke-WebRequest -Uri $Url -OutFile $Archive
  Expand-Archive -Path $Archive -DestinationPath $TempDir -Force
  $Binary = Get-ChildItem -Path $TempDir -Recurse -Filter "ikaros.exe" | Select-Object -First 1
  if ($null -eq $Binary) {
    throw "release archive did not contain ikaros.exe"
  }
  Copy-Item -Path $Binary.FullName -Destination (Join-Path $BinDir "ikaros.exe") -Force
  Write-Host "installed: $(Join-Path $BinDir "ikaros.exe")"
  Write-Host "next: ikaros setup"
  Write-Host "path_hint: add $BinDir to PATH if ikaros is not found"
} finally {
  Remove-Item -Path $TempDir -Recurse -Force -ErrorAction SilentlyContinue
}
