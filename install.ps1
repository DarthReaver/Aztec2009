# Installs Aztec 2009 for the current user (no admin needed): copies the program and examples
# to %LOCALAPPDATA%\Aztec2009 and adds Start menu and Desktop shortcuts.
# Run from the project folder or the unzipped release folder (right click > Run with PowerShell).
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$dest = Join-Path $env:LOCALAPPDATA 'Aztec2009'

$src = @((Join-Path $here 'aztec2009.exe'), (Join-Path $here 'target\release\aztec2009.exe')) |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $src) {
    if (Get-Command cargo -ErrorAction SilentlyContinue) {
        Push-Location $here; cargo build --release; Pop-Location
        $src = Join-Path $here 'target\release\aztec2009.exe'
    } else {
        Write-Host 'No aztec2009.exe and no cargo found. Install Rust from https://rustup.rs and rerun.' -ForegroundColor Red
        exit 1
    }
}

New-Item -ItemType Directory -Force -Path $dest | Out-Null
Copy-Item $src (Join-Path $dest 'aztec2009.exe') -Force
if (Test-Path (Join-Path $here 'examples')) { Copy-Item (Join-Path $here 'examples') $dest -Recurse -Force }

$shell = New-Object -ComObject WScript.Shell
$targets = @(
    (Join-Path ([Environment]::GetFolderPath('Programs')) 'Aztec 2009.lnk'),
    (Join-Path ([Environment]::GetFolderPath('Desktop')) 'Aztec 2009.lnk')
)
foreach ($t in $targets) {
    $l = $shell.CreateShortcut($t)
    $l.TargetPath = Join-Path $dest 'aztec2009.exe'
    $l.WorkingDirectory = $dest
    $l.Description = 'Maya 2009 style 3D animation tool'
    $l.Save()
}
Write-Host "Installed to $dest"
Write-Host 'Shortcuts added to the Start menu and Desktop.'
