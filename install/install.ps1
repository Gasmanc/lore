# Install the lore CLI on Windows.
#
# Usage (PowerShell):
#   irm https://raw.githubusercontent.com/lore-dev/lore/main/install/install.ps1 | iex
#
# Options (environment variables):
#   $env:LORE_VERSION  — release tag to install, e.g. "v0.1.0" (default: latest)
#   $env:LORE_BIN_DIR  — directory to place the binary (default: %LOCALAPPDATA%\lore\bin)

$ErrorActionPreference = 'Stop'

$Repo   = "lore-dev/lore"
$BinDir = if ($env:LORE_BIN_DIR) { $env:LORE_BIN_DIR } else { Join-Path $env:LOCALAPPDATA "lore\bin" }
$Target = "x86_64-pc-windows-msvc"

# ── Resolve version ────────────────────────────────────────────────────────────

$Version = $env:LORE_VERSION
if (-not $Version) {
    $Release = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest"
    $Version = $Release.tag_name
}

if (-not $Version) {
    Write-Error "Could not determine latest release. Set `$env:LORE_VERSION explicitly."
    exit 1
}

Write-Host "Installing lore $Version for $Target..."

# ── Download ───────────────────────────────────────────────────────────────────

$Archive    = "lore-$Version-$Target.zip"
$BaseUrl    = "https://github.com/$Repo/releases/download/$Version"
$Tmp        = Join-Path $env:TEMP "lore-install-$(New-Guid)"

New-Item -ItemType Directory -Path $Tmp | Out-Null

try {
    Invoke-WebRequest "$BaseUrl/$Archive"    -OutFile (Join-Path $Tmp $Archive)
    Invoke-WebRequest "$BaseUrl/SHA256SUMS"  -OutFile (Join-Path $Tmp "SHA256SUMS")

    # Verify checksum.
    $Expected = (Get-Content (Join-Path $Tmp "SHA256SUMS") | Where-Object { $_ -match [regex]::Escape($Archive) }) -split '\s+' | Select-Object -First 1
    $Actual   = (Get-FileHash (Join-Path $Tmp $Archive) -Algorithm SHA256).Hash.ToLower()
    if ($Expected -and $Actual -ne $Expected) {
        Write-Error "Checksum mismatch for $Archive"
        exit 1
    }

    # Extract and install.
    Expand-Archive (Join-Path $Tmp $Archive) -DestinationPath $Tmp
    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    Copy-Item (Join-Path $Tmp "lore-$Version-$Target\lore.exe") (Join-Path $BinDir "lore.exe") -Force

    Write-Host "lore installed to $BinDir\lore.exe"

    # Add to PATH for the current user if not already present.
    $UserPath = [Environment]::GetEnvironmentVariable("PATH", "User")
    if ($UserPath -notlike "*$BinDir*") {
        [Environment]::SetEnvironmentVariable("PATH", "$UserPath;$BinDir", "User")
        Write-Host "  Added $BinDir to your user PATH. Restart your terminal to use lore."
    }
} finally {
    Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
