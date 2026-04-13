# NTK system installer — Windows (PowerShell)
# Usage: irm https://ntk.valraw.com/install.ps1 | iex
$ErrorActionPreference = 'Stop'

$Repo   = "VALRAW-ALL/ntk"
$Latest = (Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest").tag_name
$Url    = "https://github.com/$Repo/releases/download/$Latest/ntk-windows-x86_64.exe"
$Dest   = "$env:LOCALAPPDATA\ntk\ntk.exe"

Write-Host "Downloading NTK $Latest for Windows x86_64..."
New-Item -ItemType Directory -Force -Path "$env:LOCALAPPDATA\ntk" | Out-Null
Invoke-WebRequest $Url -OutFile $Dest

# Add to user PATH if not already present.
$CurrentPath = [Environment]::GetEnvironmentVariable("PATH", "User")
if ($CurrentPath -notlike "*$env:LOCALAPPDATA\ntk*") {
    [Environment]::SetEnvironmentVariable(
        "PATH",
        "$CurrentPath;$env:LOCALAPPDATA\ntk",
        "User"
    )
    Write-Host "Added $env:LOCALAPPDATA\ntk to PATH."
}

Write-Host "NTK installed to $Dest"
Write-Host "Run: ntk init -g"
