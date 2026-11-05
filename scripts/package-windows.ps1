# Zips the Windows release build with the README, license and examples into dist/.
# Usage: scripts/package-windows.ps1   (after cargo build --release)
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches.Groups[1].Value
$name = "cocad-$version-windows-x86_64"
$stage = Join-Path "dist" $name
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item target/release/cocad.exe, README.md, LICENSE -Destination $stage
Copy-Item -Recurse examples -Destination $stage
Compress-Archive -Path $stage -DestinationPath "dist/$name.zip" -Force
Write-Host "Built dist/$name.zip"
