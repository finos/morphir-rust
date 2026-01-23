#Requires -Version 5.1
<#
.SYNOPSIS
    Morphir CLI Installer for Windows

.DESCRIPTION
    Installs the morphir CLI launcher script.

.EXAMPLE
    irm https://raw.githubusercontent.com/finos/morphir-rust/main/scripts/install.ps1 | iex

.PARAMETER NoModifyPath
    Don't add morphir to the PATH

.PARAMETER Version
    Pre-install a specific version
#>

[CmdletBinding()]
param(
    [switch]$NoModifyPath,
    [string]$Version
)

$ErrorActionPreference = "Stop"

# Configuration
$MorphirHome = if ($env:MORPHIR_HOME) { $env:MORPHIR_HOME } else { Join-Path $env:USERPROFILE ".morphir" }
$Repo = "finos/morphir-rust"
$ScriptUrl = "https://raw.githubusercontent.com/$Repo/main/scripts/morphir.ps1"

function Write-Info { Write-Host "info: $args" -ForegroundColor Blue }
function Write-Success { Write-Host "success: $args" -ForegroundColor Green }
function Write-Warning { Write-Host "warning: $args" -ForegroundColor Yellow }

# Add to user PATH
function Add-ToPath {
    param([string]$PathToAdd)

    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")

    if ($currentPath -split ";" -contains $PathToAdd) {
        Write-Info "$PathToAdd is already in PATH"
        return
    }

    $newPath = "$currentPath;$PathToAdd"
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")

    # Also update current session
    $env:Path = "$env:Path;$PathToAdd"

    Write-Success "Added $PathToAdd to PATH"
    Write-Info "Restart your terminal or run `$env:Path = [Environment]::GetEnvironmentVariable('Path', 'User') to use morphir"
}

# Main installation
function Main {
    Write-Host ""
    Write-Host "Morphir CLI Installer" -ForegroundColor Cyan
    Write-Host ""

    # Create directories
    $binDir = Join-Path $MorphirHome "bin"
    Write-Info "Creating $binDir..."
    New-Item -ItemType Directory -Path $binDir -Force | Out-Null

    # Download launcher script
    Write-Info "Downloading morphir launcher..."
    $scriptPath = Join-Path $binDir "morphir.ps1"
    Invoke-WebRequest -Uri $ScriptUrl -OutFile $scriptPath -UseBasicParsing

    # Create batch wrapper for easier invocation
    $batchPath = Join-Path $binDir "morphir.cmd"
    @"
@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0morphir.ps1" %*
"@ | Out-File -FilePath $batchPath -Encoding ASCII

    # Add to PATH
    if (-not $NoModifyPath) {
        Add-ToPath $binDir
    }

    # Pre-install version if specified
    if ($Version) {
        Write-Info "Pre-installing morphir $Version..."
        & $scriptPath self install $Version
    }

    Write-Host ""
    Write-Success "Morphir installed successfully!"
    Write-Host ""
    Write-Host "To get started, run:"
    Write-Host ""
    Write-Host "  morphir --help" -ForegroundColor Blue
    Write-Host ""
    Write-Host "The first time you run a morphir command, it will automatically"
    Write-Host "download the latest version."
    Write-Host ""
    Write-Host "Useful commands:"
    Write-Host "  morphir self upgrade    # Update to latest version" -ForegroundColor Blue
    Write-Host "  morphir self list       # List installed versions" -ForegroundColor Blue
    Write-Host "  morphir self which      # Show current version" -ForegroundColor Blue
    Write-Host ""
}

Main
