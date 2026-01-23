#Requires -Version 5.1
<#
.SYNOPSIS
    Morphir CLI launcher script for Windows

.DESCRIPTION
    This script automatically downloads and runs the correct version of morphir.
    It supports version pinning, multiple installation backends, and self-management.

.EXAMPLE
    morphir ir migrate --input ./morphir-ir.json --output ./v4.json

.EXAMPLE
    morphir +0.1.0 schema --output ./schema.json

.EXAMPLE
    morphir self upgrade
#>

[CmdletBinding()]
param(
    [Parameter(Position = 0, ValueFromRemainingArguments = $true)]
    [string[]]$Arguments
)

$ErrorActionPreference = "Stop"

# Configuration
$script:MorphirHome = if ($env:MORPHIR_HOME) { $env:MORPHIR_HOME } else { Join-Path $env:USERPROFILE ".morphir" }
$script:Repo = "finos/morphir-rust"
$script:GitHubApi = "https://api.github.com/repos/$Repo"
$script:GitHubReleases = "https://github.com/$Repo/releases"
$script:CacheTTL = 86400  # 24 hours in seconds

# Logging functions
function Write-Info { Write-Host "info: $args" -ForegroundColor Blue }
function Write-Success { Write-Host "success: $args" -ForegroundColor Green }
function Write-Warning { Write-Host "warning: $args" -ForegroundColor Yellow }
function Write-Error { Write-Host "error: $args" -ForegroundColor Red }

# Detect platform
function Get-Platform {
    $arch = if ([System.Environment]::Is64BitOperatingSystem) {
        if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq "Arm64") {
            "aarch64"
        } else {
            "x86_64"
        }
    } else {
        "x86_64"
    }

    return "$arch-pc-windows-msvc"
}

# Find .morphir-version file
function Find-VersionFile {
    $dir = Get-Location
    while ($dir) {
        $versionFile = Join-Path $dir ".morphir-version"
        if (Test-Path $versionFile) {
            return $versionFile
        }
        $parent = Split-Path $dir -Parent
        if ($parent -eq $dir) { break }
        $dir = $parent
    }
    return $null
}

# Find morphir.toml and extract version
function Find-TomlVersion {
    $dir = Get-Location
    while ($dir) {
        $tomlFile = Join-Path $dir "morphir.toml"
        if (Test-Path $tomlFile) {
            $content = Get-Content $tomlFile -Raw
            # Match: version = "0.1.0" or morphir-version = "0.1.0"
            if ($content -match '(?:morphir-)?version\s*=\s*[''"]([^''"]+)[''"]') {
                return $Matches[1]
            }
        }
        $parent = Split-Path $dir -Parent
        if ($parent -eq $dir) { break }
        $dir = $parent
    }
    return $null
}

# Get latest version from GitHub
function Get-LatestVersion {
    $cacheFile = Join-Path $MorphirHome ".latest"
    $cacheTimeFile = Join-Path $MorphirHome ".latest-time"
    $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()

    # Check cache
    if ((Test-Path $cacheFile) -and (Test-Path $cacheTimeFile)) {
        $cachedTime = [long](Get-Content $cacheTimeFile)
        if (($now - $cachedTime) -lt $CacheTTL) {
            return Get-Content $cacheFile
        }
    }

    # Fetch from GitHub
    Write-Info "Fetching latest version from GitHub..."
    $response = Invoke-RestMethod -Uri "$GitHubApi/releases/latest" -Headers @{ "User-Agent" = "morphir-launcher" }
    $version = $response.tag_name -replace "^v", ""

    # Cache result
    if (-not (Test-Path $MorphirHome)) {
        New-Item -ItemType Directory -Path $MorphirHome -Force | Out-Null
    }
    $version | Out-File -FilePath $cacheFile -NoNewline
    $now.ToString() | Out-File -FilePath $cacheTimeFile -NoNewline

    return $version
}

# Resolve version
function Resolve-Version {
    param([string]$Override)

    if ($Override) {
        return $Override -replace "^v", ""
    }

    if ($env:MORPHIR_VERSION) {
        return $env:MORPHIR_VERSION -replace "^v", ""
    }

    $versionFile = Find-VersionFile
    if ($versionFile) {
        return (Get-Content $versionFile).Trim() -replace "^v", ""
    }

    $tomlVersion = Find-TomlVersion
    if ($tomlVersion) {
        return $tomlVersion -replace "^v", ""
    }

    return Get-LatestVersion
}

# Check if version is installed
function Test-Installed {
    param([string]$Version)
    $binary = Join-Path $MorphirHome "versions" $Version "morphir-bin.exe"
    return Test-Path $binary
}

# Get binary path
function Get-BinaryPath {
    param([string]$Version)
    return Join-Path $MorphirHome "versions" $Version "morphir-bin.exe"
}

# Check if command exists
function Test-Command {
    param([string]$Command)
    return $null -ne (Get-Command $Command -ErrorAction SilentlyContinue)
}

# Detect backend
function Get-Backend {
    $forced = $env:MORPHIR_BACKEND
    if ($forced) { return $forced }

    if (Test-Command "mise") { return "mise" }
    if (Test-Command "cargo-binstall") { return "binstall" }
    return "github"
}

# Install from GitHub releases
function Install-FromGitHub {
    param([string]$Version)

    $platform = Get-Platform
    Write-Info "Installing morphir $Version from GitHub releases..."

    $archiveName = "morphir-$Version-$platform.zip"
    $url = "$GitHubReleases/download/v$Version/$archiveName"

    $tempDir = Join-Path $env:TEMP "morphir-install-$(Get-Random)"
    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

    try {
        $archivePath = Join-Path $tempDir $archiveName
        Write-Info "Downloading $url"
        Invoke-WebRequest -Uri $url -OutFile $archivePath -UseBasicParsing

        # Extract
        Expand-Archive -Path $archivePath -DestinationPath $tempDir -Force

        # Move binary
        $versionDir = Join-Path $MorphirHome "versions" $Version
        New-Item -ItemType Directory -Path $versionDir -Force | Out-Null

        $sourceBinary = Join-Path $tempDir "morphir.exe"
        if (-not (Test-Path $sourceBinary)) {
            # Try without extension
            $sourceBinary = Join-Path $tempDir "morphir"
        }

        $destBinary = Join-Path $versionDir "morphir-bin.exe"
        Move-Item -Path $sourceBinary -Destination $destBinary -Force

        Write-Success "Installed morphir $Version"
    }
    finally {
        Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# Install using cargo-binstall
function Install-FromBinstall {
    param([string]$Version)

    Write-Info "Installing morphir $Version using cargo-binstall..."

    $tempDir = Join-Path $env:TEMP "morphir-install-$(Get-Random)"
    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

    try {
        $env:CARGO_HOME = $tempDir
        & cargo-binstall --git "https://github.com/$Repo" --tag "v$Version" --no-confirm --root $tempDir morphir

        $versionDir = Join-Path $MorphirHome "versions" $Version
        New-Item -ItemType Directory -Path $versionDir -Force | Out-Null

        $sourceBinary = Join-Path $tempDir "bin" "morphir.exe"
        $destBinary = Join-Path $versionDir "morphir-bin.exe"
        Move-Item -Path $sourceBinary -Destination $destBinary -Force

        Write-Success "Installed morphir $Version"
    }
    finally {
        Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item Env:\CARGO_HOME -ErrorAction SilentlyContinue
    }
}

# Install version
function Install-Version {
    param([string]$Version)

    $backend = Get-Backend

    switch ($backend) {
        "binstall" { Install-FromBinstall $Version }
        default { Install-FromGitHub $Version }
    }
}

# Ensure version is installed
function Ensure-Installed {
    param([string]$Version)

    if (-not (Test-Installed $Version)) {
        Install-Version $Version
    }
}

# Handle self commands
function Invoke-SelfCommand {
    param([string[]]$Args)

    $cmd = if ($Args.Count -gt 0) { $Args[0] } else { "help" }
    $remaining = if ($Args.Count -gt 1) { $Args[1..($Args.Count - 1)] } else { @() }

    switch ($cmd) {
        "upgrade" {
            Write-Info "Checking for updates..."
            Remove-Item (Join-Path $MorphirHome ".latest") -ErrorAction SilentlyContinue
            Remove-Item (Join-Path $MorphirHome ".latest-time") -ErrorAction SilentlyContinue
            $latest = Get-LatestVersion
            Write-Info "Latest version: $latest"

            if (Test-Installed $latest) {
                Write-Info "Version $latest is already installed"
            }
            else {
                Install-Version $latest
            }
        }

        "list" {
            Write-Host "Installed versions:"
            $versionsDir = Join-Path $MorphirHome "versions"
            if (Test-Path $versionsDir) {
                Get-ChildItem -Path $versionsDir -Directory | ForEach-Object {
                    Write-Host "  $($_.Name)"
                }
            }
            else {
                Write-Host "  (none)"
            }
        }

        "which" {
            $version = Resolve-Version
            $binary = Get-BinaryPath $version

            Write-Host "Version: $version"
            if (Test-Installed $version) {
                Write-Host "Binary: $binary"
                Write-Host "Status: installed"
            }
            else {
                Write-Host "Binary: $binary (not installed)"
                Write-Host "Status: will download on first use"
            }
            Write-Host "Backend: $(Get-Backend)"
        }

        "install" {
            if ($remaining.Count -eq 0) {
                Write-Error "Usage: morphir self install <version>"
                exit 1
            }
            $version = $remaining[0] -replace "^v", ""
            Install-Version $version
        }

        "prune" {
            $current = Resolve-Version
            Write-Info "Current version: $current (keeping)"

            $versionsDir = Join-Path $MorphirHome "versions"
            if (Test-Path $versionsDir) {
                Get-ChildItem -Path $versionsDir -Directory | Where-Object { $_.Name -ne $current } | ForEach-Object {
                    Write-Info "Removing $($_.Name)..."
                    Remove-Item -Path $_.FullName -Recurse -Force
                }
            }
            Write-Success "Pruned old versions"
        }

        "update" {
            Write-Info "Updating morphir launcher script..."
            $scriptUrl = "https://raw.githubusercontent.com/$Repo/main/scripts/morphir.ps1"
            $scriptPath = $MyInvocation.PSCommandPath

            Invoke-WebRequest -Uri $scriptUrl -OutFile "$scriptPath.new" -UseBasicParsing
            Move-Item -Path "$scriptPath.new" -Destination $scriptPath -Force
            Write-Success "Updated launcher script"
        }

        default {
            Write-Host @"
morphir self - Manage the morphir installation

Commands:
  upgrade          Download and install the latest version
  list             List installed versions
  which            Show which version would be used
  install <ver>    Install a specific version
  prune            Remove old versions (keeps current)
  update           Update this launcher script

Environment variables:
  MORPHIR_VERSION  Override version to use
  MORPHIR_BACKEND  Force backend: mise, binstall, github, cargo
  MORPHIR_HOME     Override home directory (default: ~/.morphir)
"@
        }
    }
}

# Main
function Main {
    # Create home directory
    if (-not (Test-Path $MorphirHome)) {
        New-Item -ItemType Directory -Path $MorphirHome -Force | Out-Null
    }

    # Parse arguments
    $versionOverride = ""
    $args = @()

    foreach ($arg in $Arguments) {
        if ($arg -match "^\+(.+)$") {
            $versionOverride = $Matches[1] -replace "^v", ""
        }
        elseif ($arg -eq "self" -and $args.Count -eq 0) {
            # Handle self subcommand
            $remaining = $Arguments | Select-Object -Skip ($Arguments.IndexOf("self") + 1)
            Invoke-SelfCommand $remaining
            return
        }
        else {
            $args += $arg
        }
    }

    # Resolve version
    $version = Resolve-Version $versionOverride

    # Ensure installed
    Ensure-Installed $version

    # Run morphir
    $binary = Get-BinaryPath $version
    & $binary @args
}

Main
