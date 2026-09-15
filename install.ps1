# twr installer (Windows PowerShell) - https://github.com/quangdang46/twitter_cli
#
# Usage:
#   irm "https://raw.githubusercontent.com/quangdang46/twitter_cli/main/install.ps1" | iex
#   & ([scriptblock]::Create((irm "...install.ps1"))) -EasyMode -Verify
#   & ([scriptblock]::Create((irm "...install.ps1"))) -Version v0.1.0

param(
    [string]$Dest = "$env:LOCALAPPDATA\Programs\twr",
    [string]$Version = "",
    [switch]$EasyMode,
    [switch]$Verify,
    [switch]$FromSource,
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"
$BinaryName = "twr"
$BinaryFile = "$BinaryName.exe"
$Owner = "quangdang46"
$Repo = "twitter_cli"

function Write-Info    { param($m) Write-Host "[$BinaryName] $m" }
function Write-Ok      { param($m) Write-Host "OK: $m" -ForegroundColor Green }
function Write-Warn    { param($m) Write-Host "WARN: $m" -ForegroundColor Yellow }
function Die           { param($m) Write-Host "ERROR: $m" -ForegroundColor Red; exit 1 }

if ($Uninstall) {
    $target = Join-Path $Dest $BinaryFile
    if (Test-Path $target) { Remove-Item $target -Force }
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($userPath -and $userPath -like "*$Dest*") {
        $newPath = ($userPath -split ";" | Where-Object { $_ -ne $Dest }) -join ";"
        [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    }
    Write-Ok "$BinaryName uninstalled"
    exit 0
}

function Get-Platform {
    $arch = if ([Environment]::Is64BitOperatingSystem) { "x86_64" } else { Die "Unsupported 32-bit Windows" }
    return "windows_$arch"
}

function Resolve-Version {
    if ($Version) { return }
    try {
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Owner/$Repo/releases/latest" -TimeoutSec 30
        $script:Version = $release.tag_name
    } catch {
        try {
            $resp = Invoke-WebRequest -Uri "https://github.com/$Owner/$Repo/releases/latest" -MaximumRedirection 0 -ErrorAction SilentlyContinue
        } catch {
            $resp = $_.Exception.Response
        }
        if ($resp -and $resp.Headers.Location) {
            $script:Version = ($resp.Headers.Location -split "/tag/")[-1]
        }
    }
    if (-not $Version -or $Version -notmatch "^v[0-9]") {
        Die "Could not resolve the latest version - pass -Version vX.Y.Z or -FromSource"
    }
    Write-Info "Latest release: $Version"
}

function Invoke-DownloadWithRetry {
    param([string]$Url, [string]$OutFile, [int]$MaxRetries = 3)
    for ($i = 1; $i -le $MaxRetries; $i++) {
        try {
            Invoke-WebRequest -Uri $Url -OutFile $OutFile -TimeoutSec 120 -ErrorAction Stop
            return $true
        } catch {
            if ($i -lt $MaxRetries) {
                Write-Warn "Download failed (attempt $i/$MaxRetries), retrying in 3s..."
                Start-Sleep -Seconds 3
            }
        }
    }
    return $false
}

function Build-FromSource {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Die "Rust/cargo not found. Install it: https://rustup.rs"
    }
    Write-Info "Building from source (this may take a few minutes)..."
    $src = Join-Path $env:TEMP "twr-src-$PID"
    git clone --depth 1 "https://github.com/$Owner/$Repo.git" $src
    Push-Location $src
    try {
        cargo build --release -p $BinaryName
        New-Item -ItemType Directory -Force -Path $Dest | Out-Null
        Copy-Item "target\release\$BinaryFile" (Join-Path $Dest $BinaryFile) -Force
    } finally {
        Pop-Location
    }
}

function Override-StalePathCopy {
    $existing = Get-Command $BinaryName -ErrorAction SilentlyContinue
    if (-not $existing) { return }
    $existingDir = (Split-Path $existing.Source -Parent).TrimEnd('\').ToLower()
    $destDir = $Dest.TrimEnd('\').ToLower()
    if ($existingDir -eq $destDir) { return }

    $dstFile = $existing.Source
    $oldFile = "$dstFile.old.$PID"
    try {
        Rename-Item -LiteralPath $dstFile -NewName $oldFile -ErrorAction SilentlyContinue
        Copy-Item -LiteralPath (Join-Path $Dest $BinaryFile) -Destination $dstFile -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $dstFile) {
            Write-Ok "replaced $dstFile"
            Remove-Item -LiteralPath $oldFile -Force -ErrorAction SilentlyContinue
        } else {
            $inner = "Copy-Item -LiteralPath '$(Join-Path $Dest $BinaryFile)' -Destination '$dstFile' -Force"
            Start-Process powershell -ArgumentList @('-NoProfile', '-Command', $inner) -Verb RunAs -Wait | Out-Null
            if (Test-Path -LiteralPath $dstFile) {
                Write-Ok "replaced $dstFile (elevated)"
            } else {
                Move-Item -LiteralPath $oldFile -Destination $dstFile -Force -ErrorAction SilentlyContinue
                Write-Warn "could not update $dstFile - run as admin or remove it manually"
            }
        }
    } catch {
        Write-Warn "could not update $dstFile - run as admin or remove it manually"
    }
}

function Add-ToPath {
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($userPath -and $userPath -like "*$Dest*") { return }
    if ($EasyMode) {
        $newPath = "$Dest;$userPath"
        [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
        $env:Path = "$Dest;$env:Path"
        Write-Warn "PATH updated - restart your terminal for it to take effect everywhere"
    } else {
        Write-Warn "Add to PATH: `$env:Path = `"$Dest;`$env:Path`" (or re-run with -EasyMode)"
    }
}

# === Main ===
New-Item -ItemType Directory -Force -Path $Dest | Out-Null
$platform = Get-Platform
Write-Info "Platform: $platform | Dest: $Dest"

if (-not $FromSource) {
    Resolve-Version
    $archive = "$BinaryName-windows-x86_64.zip"
    $url = "https://github.com/$Owner/$Repo/releases/download/$Version/$archive"
    $tmpDir = Join-Path $env:TEMP "twr-install-$PID"
    New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null
    $archivePath = Join-Path $tmpDir $archive

    if (Invoke-DownloadWithRetry -Url $url -OutFile $archivePath) {
        $shaUrl = "$url.sha256"
        $shaPath = "$archivePath.sha256"
        if (Invoke-DownloadWithRetry -Url $shaUrl -OutFile $shaPath -MaxRetries 1) {
            $expected = (Get-Content $shaPath | Select-Object -First 1).Split(" ")[0].Trim()
            $actual = (Get-FileHash -Algorithm SHA256 $archivePath).Hash.ToLower()
            if ($expected -ne $actual) { Die "Checksum mismatch for $archive - aborting" }
            Write-Info "Checksum verified"
        } else {
            Write-Warn "No checksum sidecar found for $archive - skipping verification"
        }
        Expand-Archive -Path $archivePath -DestinationPath $tmpDir -Force
        $binPath = Get-ChildItem -Path $tmpDir -Filter $BinaryFile -Recurse | Select-Object -First 1
        if (-not $binPath) { Die "Binary not found inside the downloaded archive" }
        Copy-Item $binPath.FullName (Join-Path $Dest $BinaryFile) -Force
    } else {
        Write-Warn "Binary download failed - falling back to building from source"
        Build-FromSource
    }
    Remove-Item $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
} else {
    Build-FromSource
}

Override-StalePathCopy
Add-ToPath

if ($Verify) {
    & (Join-Path $Dest $BinaryFile) --version
}

Write-Host ""
Write-Host "OK: $BinaryName installed -> $(Join-Path $Dest $BinaryFile)" -ForegroundColor Green
Write-Host "  Quick start:"
Write-Host "    $BinaryName status --json"
Write-Host ""
Write-Host "  Note: twr is pre-implementation (see PLAN.md) - status/schema are"
Write-Host "  scaffold stubs today, not real network calls."
