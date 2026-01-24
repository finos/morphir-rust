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

# Dev mode: when enabled, runs from local source instead of downloaded binary
# Can be enabled via:
#   - MORPHIR_DEV=1 environment variable
#   - --dev command-line flag
#   - "local-dev" in .morphir-version file
#   - dev_mode = true in morphir.toml [morphir] section
# MORPHIR_DEV_PATH can specify the source repository path (default: auto-detect)

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

# Check if dev mode is enabled via morphir.toml
function Find-TomlDevMode {
    $dir = Get-Location
    while ($dir) {
        $tomlFile = Join-Path $dir "morphir.toml"
        if (Test-Path $tomlFile) {
            $content = Get-Content $tomlFile -Raw
            if ($content -match 'dev_mode\s*=\s*(true|1|yes)') {
                return $dir
            }
        }
        $parent = Split-Path $dir -Parent
        if ($parent -eq $dir) { break }
        $dir = $parent
    }
    return $null
}

# Find the morphir-rust source directory for dev mode
function Find-DevSourceDir {
    # Helper function to check if a directory is the morphir-rust repo
    function Test-MorphirRepo {
        param([string]$Dir)

        if (-not $Dir -or -not (Test-Path $Dir)) {
            return $false
        }

        $cargoToml = Join-Path $Dir "Cargo.toml"
        $cratesDir = Join-Path $Dir "crates\morphir"

        # Check for workspace Cargo.toml with crates/morphir
        if ((Test-Path $cargoToml) -and (Test-Path $cratesDir)) {
            $content = Get-Content $cargoToml -Raw -ErrorAction SilentlyContinue
            if ($content -match '\[workspace\]') {
                return $true
            }
        }

        # Also check for direct morphir package
        if (Test-Path $cargoToml) {
            $content = Get-Content $cargoToml -Raw -ErrorAction SilentlyContinue
            if ($content -match 'name\s*=\s*"morphir"') {
                return $true
            }
        }

        return $false
    }

    # 1. Check MORPHIR_DEV_PATH environment variable
    if ($env:MORPHIR_DEV_PATH -and (Test-Path $env:MORPHIR_DEV_PATH)) {
        if (Test-MorphirRepo $env:MORPHIR_DEV_PATH) {
            return $env:MORPHIR_DEV_PATH
        }
    }

    # 2. Check CI environment variables
    $ciLocations = @(
        $env:GITHUB_WORKSPACE,        # GitHub Actions
        $env:CI_PROJECT_DIR,          # GitLab CI
        $env:WORKSPACE,               # Jenkins
        $env:BITBUCKET_CLONE_DIR,     # Bitbucket Pipelines
        $env:CIRCLE_WORKING_DIRECTORY, # CircleCI
        $env:TRAVIS_BUILD_DIR,        # Travis CI
        $env:BUILD_SOURCESDIRECTORY   # Azure DevOps
    )

    foreach ($ciLoc in $ciLocations) {
        if (Test-MorphirRepo $ciLoc) {
            return $ciLoc
        }
    }

    # 3. Check if current directory or parent is the source repo
    $dir = Get-Location
    while ($dir) {
        if (Test-MorphirRepo $dir) {
            return $dir.ToString()
        }
        $parent = Split-Path $dir -Parent
        if ($parent -eq $dir) { break }
        $dir = $parent
    }

    # 4. Check common development locations (local dev)
    $commonLocations = @(
        "$env:USERPROFILE\code\morphir-rust",
        "$env:USERPROFILE\dev\morphir-rust",
        "$env:USERPROFILE\src\morphir-rust",
        "$env:USERPROFILE\projects\morphir-rust",
        "$env:USERPROFILE\repos\finos\morphir-rust",
        "$env:USERPROFILE\code\repos\github\finos\morphir-rust"
    )

    foreach ($loc in $commonLocations) {
        if (Test-MorphirRepo $loc) {
            return $loc
        }
    }

    return $null
}

# Check if we should run in dev mode
function Test-DevMode {
    param([bool]$CliDevFlag)

    # 1. CLI --dev flag
    if ($CliDevFlag) { return $true }

    # 2. MORPHIR_DEV environment variable
    if ($env:MORPHIR_DEV -eq "1" -or $env:MORPHIR_DEV -eq "true") {
        return $true
    }

    # 3. Check .morphir-version for "local-dev"
    $versionFile = Find-VersionFile
    if ($versionFile) {
        $content = (Get-Content $versionFile).Trim()
        if ($content -eq "local-dev") {
            return $true
        }
    }

    # 4. Check morphir.toml for dev_mode = true
    if (Find-TomlDevMode) {
        return $true
    }

    return $false
}

# Run morphir in dev mode (from source)
function Invoke-DevMode {
    param([string[]]$Args)

    $sourceDir = Find-DevSourceDir
    if (-not $sourceDir) {
        Write-Error "Dev mode enabled but cannot find morphir-rust source directory"
        Write-Error "Set MORPHIR_DEV_PATH to the morphir-rust repository path"
        exit 1
    }

    Write-Info "Running in dev mode from: $sourceDir"

    # Check if we have a pre-built debug binary
    $debugBinary = Join-Path $sourceDir "target\debug\morphir.exe"
    $releaseBinary = Join-Path $sourceDir "target\release\morphir.exe"

    if (Test-Path $debugBinary) {
        # Check if source files are newer than binary
        $binaryTime = (Get-Item $debugBinary).LastWriteTime
        $newerSources = Get-ChildItem -Path (Join-Path $sourceDir "crates") -Filter "*.rs" -Recurse |
            Where-Object { $_.LastWriteTime -gt $binaryTime } |
            Select-Object -First 1

        if (-not $newerSources) {
            Write-Info "Using cached debug binary"
            & $debugBinary @Args
            return
        }
    }

    # Build and run with cargo
    Write-Info "Building and running with cargo..."
    Push-Location $sourceDir
    try {
        & cargo run --bin morphir -- @Args
    }
    finally {
        Pop-Location
    }
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

        "dev" {
            Write-Info "Dev mode status:"
            Write-Host ""

            $devEnabled = $false

            # Check MORPHIR_DEV env
            if ($env:MORPHIR_DEV -eq "1" -or $env:MORPHIR_DEV -eq "true") {
                Write-Host "  MORPHIR_DEV env:     enabled"
                $devEnabled = $true
            }
            else {
                Write-Host "  MORPHIR_DEV env:     not set"
            }

            # Check .morphir-version
            $versionFile = Find-VersionFile
            if ($versionFile) {
                $content = (Get-Content $versionFile).Trim()
                if ($content -eq "local-dev") {
                    Write-Host "  .morphir-version:    local-dev (enabled)"
                    $devEnabled = $true
                }
                else {
                    Write-Host "  .morphir-version:    $content"
                }
            }
            else {
                Write-Host "  .morphir-version:    not found"
            }

            # Check morphir.toml
            $tomlDir = Find-TomlDevMode
            if ($tomlDir) {
                Write-Host "  morphir.toml:        dev_mode=true at $tomlDir"
                $devEnabled = $true
            }
            else {
                Write-Host "  morphir.toml:        dev_mode not set"
            }

            # Check MORPHIR_DEV_PATH
            if ($env:MORPHIR_DEV_PATH) {
                Write-Host "  MORPHIR_DEV_PATH:    $env:MORPHIR_DEV_PATH"
            }
            else {
                Write-Host "  MORPHIR_DEV_PATH:    not set (will auto-detect)"
            }

            Write-Host ""
            $sourceDir = Find-DevSourceDir
            if ($sourceDir) {
                Write-Host "  Source directory:    $sourceDir"

                $debugBinary = Join-Path $sourceDir "target\debug\morphir.exe"
                $releaseBinary = Join-Path $sourceDir "target\release\morphir.exe"

                if (Test-Path $debugBinary) {
                    Write-Host "  Debug binary:        $debugBinary (available)"
                }
                else {
                    Write-Host "  Debug binary:        not built"
                }
                if (Test-Path $releaseBinary) {
                    Write-Host "  Release binary:      $releaseBinary (available)"
                }
                else {
                    Write-Host "  Release binary:      not built"
                }
            }
            else {
                Write-Host "  Source directory:    not found"
            }

            Write-Host ""
            if ($devEnabled) {
                Write-Host "Dev mode is ENABLED" -ForegroundColor Green
            }
            else {
                Write-Host "Dev mode is DISABLED" -ForegroundColor Yellow
            }

            Write-Host ""
            Write-Host "To enable dev mode, use one of:"
            Write-Host "  - morphir --dev <command>        (one-time)"
            Write-Host '  - $env:MORPHIR_DEV = "1"         (session)'
            Write-Host "  - Set-Content .morphir-version 'local-dev'  (project)"
            Write-Host "  - Add 'dev_mode = true' to morphir.toml [morphir] section"
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
  dev              Show dev mode status and configuration

Environment variables:
  MORPHIR_VERSION  Override version to use
  MORPHIR_BACKEND  Force backend: mise, binstall, github, cargo
  MORPHIR_HOME     Override home directory (default: ~/.morphir)
  MORPHIR_DEV      Set to 1 to enable dev mode (run from source)
  MORPHIR_DEV_PATH Path to morphir-rust source directory

Dev mode:
  Use --dev flag or set MORPHIR_DEV=1 to run from local source.
  Put "local-dev" in .morphir-version or dev_mode=true in morphir.toml.
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
    $devFlag = $false
    $args = @()

    foreach ($arg in $Arguments) {
        if ($arg -match "^\+(.+)$") {
            $versionOverride = $Matches[1] -replace "^v", ""
        }
        elseif ($arg -eq "--dev") {
            $devFlag = $true
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

    # Check if we should run in dev mode
    if (Test-DevMode $devFlag) {
        Invoke-DevMode $args
        return
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
