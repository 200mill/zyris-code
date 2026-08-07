# zyris-code installer for Windows (PowerShell 5.1+ and 7).
#
# Builds zyris-code from source with cargo and installs the binary into
# <prefix>\bin. Re-running the script rebuilds from the latest main, so it
# doubles as the updater.
#
# Usage:
#   irm https://raw.githubusercontent.com/attacca-cc/zyris-code/main/install.ps1 | iex
#   .\install.ps1 [-Prefix <dir>] [-Uninstall] [-Help]

$RepoUrl = "https://github.com/attacca-cc/zyris-code"

function Install-ZyrisCode {
  param(
    [string]$Prefix = "",
    [switch]$Uninstall,
    [switch]$Help
  )

  if ($Help) {
    @"
Usage: install.ps1 [-Prefix <dir>] [-Uninstall] [-Help]

Installs the zyris-code terminal client by building it from source with cargo.

  -Prefix <dir>  Install root; the binary lands in <dir>\bin.
                 Default: the directory that contains the cargo bin dir
                 (the cargo home, usually %USERPROFILE%\.cargo).
  -Uninstall     Remove the zyris-code binary.
  -Help          Show this help.

The binary is installed to <prefix>\bin\zyris-code.exe. Re-running this
script rebuilds and reinstalls the latest version from main.
"@
    return
  }

  if (-not $Prefix) {
    $cargoCmd = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cargoCmd) {
      # Parent of the directory cargo lives in, so the binary lands right
      # back in the bin dir that is already on PATH (~/.cargo\bin).
      $Prefix = Split-Path (Split-Path $cargoCmd.Source -Parent) -Parent
    } else {
      $Prefix = Join-Path $HOME ".cargo"
    }
  }
  $BinDir = Join-Path $Prefix "bin"

  if ($Uninstall) {
    $target = Join-Path $BinDir "zyris-code.exe"
    if (Test-Path -LiteralPath $target) {
      Remove-Item -LiteralPath $target -Force
      Write-Host "Uninstalled zyris-code from $BinDir"
    } else {
      Write-Error "zyris-code is not installed in $BinDir"
    }
    return
  }

  if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error @"
cargo (Rust) is required to build zyris-code from source.

Install it with:
    winget install Rustlang.Rustup
or from https://rustup.rs

then open a new terminal and re-run this script.
"@
    return
  }
  if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Write-Error "git is required to fetch the zyris-code sources. Install it from https://git-scm.com and re-run this script."
    return
  }

  Write-Host "Installing zyris-code ..." -ForegroundColor Cyan
  Write-Host "  source:  $RepoUrl"
  Write-Host "  prefix:  $Prefix  (binaries go to $BinDir)"
  Write-Host "  cargo:   $((Get-Command cargo).Source)"

  New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
  & cargo install --git $RepoUrl --locked --force --root $Prefix
  if ($LASTEXITCODE -ne 0) {
    Write-Error "cargo install failed with exit code $LASTEXITCODE"
    return
  }

  Write-Host ""
  Write-Host "Installed zyris-code to $BinDir\zyris-code.exe" -ForegroundColor Green
  Write-Host ""
  Write-Host "Run it from the directory you want to work in:"
  Write-Host "    cd C:\path\to\your-project"
  Write-Host "    zyris-code"
}

# Parse arguments manually so the script works both when run directly
# (.\install.ps1 -Prefix ...) and when piped through iex (irm ... | iex),
# where a top-level param() block is not allowed.
$prefixArg = ""
$uninstallArg = $false
$helpArg = $false
$badArgs = $false
$i = 0
while ($i -lt $args.Count) {
  switch ($args[$i]) {
    "-Prefix" {
      if ($i + 1 -ge $args.Count) {
        Write-Error "-Prefix requires a value"
        $badArgs = $true
        break
      }
      $prefixArg = $args[$i + 1]
      $i += 2
    }
    "-Uninstall" {
      $uninstallArg = $true
      $i++
    }
    "-Help" {
      $helpArg = $true
      $i++
    }
    default {
      Write-Error "Unknown option: $($args[$i])"
      $badArgs = $true
      break
    }
  }
}

if (-not $badArgs) {
  Install-ZyrisCode -Prefix $prefixArg -Uninstall:$uninstallArg -Help:$helpArg
}
