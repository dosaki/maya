# Installs the latest Maya release on Windows.
#
#   irm https://raw.githubusercontent.com/dosaki/maya/main/install.ps1 | iex
#
# Environment:
#   MAYA_REPO  GitHub repo to install from (default dosaki/maya)
#
# The installer is the release's NSIS setup, run silently for the current
# user (no administrator rights), into %LOCALAPPDATA%\Maya. Run the same
# command again to update.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$repo = if ($env:MAYA_REPO) { $env:MAYA_REPO } else { 'dosaki/maya' }

function Fail($message) {
    Write-Host "maya install: $message" -ForegroundColor Red
    exit 1
}

if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
    Fail "only 64-bit x86 Windows builds are published today (this is $env:PROCESSOR_ARCHITECTURE)."
}

try {
    $release = Invoke-RestMethod -UseBasicParsing "https://api.github.com/repos/$repo/releases/latest"
} catch {
    Fail "could not reach GitHub releases for $repo"
}
$asset = $release.assets | Where-Object { $_.name -like '*_x64-setup.exe' } | Select-Object -First 1
if (-not $asset) { Fail "no Windows build found in the latest release of $repo" }

$setup = Join-Path ([System.IO.Path]::GetTempPath()) $asset.name
Write-Host "Downloading $($asset.browser_download_url)"
Invoke-WebRequest -UseBasicParsing $asset.browser_download_url -OutFile $setup
# The build is not signed: without this, SmartScreen stops the silent install.
Unblock-File $setup

$running = Get-Process -Name maya -ErrorAction SilentlyContinue
if ($running) {
    Write-Host 'Maya is running; quitting it before replacing the app.'
    $running | ForEach-Object { $_.CloseMainWindow() | Out-Null }
    Start-Sleep -Seconds 2
    Get-Process -Name maya -ErrorAction SilentlyContinue | Stop-Process -Force
}

$p = Start-Process -FilePath $setup -ArgumentList '/S' -Wait -PassThru
Remove-Item $setup -ErrorAction SilentlyContinue
if ($p.ExitCode -ne 0) { Fail "the installer exited with $($p.ExitCode)" }

Write-Host "Installed Maya $($release.tag_name). Start it from the Start menu."
