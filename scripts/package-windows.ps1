# PromptMan Windows Packaging Script
param(
    [string]$Version = "1.0.0",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$RootDir = Split-Path -Parent $PSScriptRoot
Set-Location $RootDir

Write-Host "========================================" -ForegroundColor Cyan
Write-Host " Packaging PromptMan v$Version for Windows" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan

if (-not $SkipBuild) {
    Write-Host "[1/4] Building release binaries..." -ForegroundColor Yellow
    cargo build --release
}

$ReleaseDir = Join-Path $RootDir "target\release"
$PromptmanExe = Join-Path $ReleaseDir "promptman.exe"
$PromptmanCliExe = Join-Path $ReleaseDir "promptman-cli.exe"

if (-not (Test-Path $PromptmanExe)) {
    throw "promptman.exe not found in $ReleaseDir"
}
if (-not (Test-Path $PromptmanCliExe)) {
    throw "promptman-cli.exe not found in $ReleaseDir"
}

$DistDir = Join-Path $RootDir "dist"
$PackageName = "PromptMan-v$Version-windows-x64"
$PackageDir = Join-Path $DistDir $PackageName
$ZipFile = Join-Path $DistDir "$PackageName.zip"

Write-Host "[2/4] Assembling release folder..." -ForegroundColor Yellow
if (Test-Path $PackageDir) {
    Remove-Item -Recurse -Force $PackageDir
}
New-Item -ItemType Directory -Force -Path $PackageDir | Out-Null

Copy-Item $PromptmanExe -Destination (Join-Path $PackageDir "promptman.exe")
Copy-Item $PromptmanCliExe -Destination (Join-Path $PackageDir "promptman-cli.exe")
Copy-Item (Join-Path $RootDir "LICENSE") -Destination (Join-Path $PackageDir "LICENSE.txt")
Copy-Item (Join-Path $RootDir "assets\icons\icon.ico") -Destination (Join-Path $PackageDir "icon.ico")

$ReadmeContent = @"
========================================================================
 PromptMan v$Version - Desktop LLM Workbench
========================================================================

Thank you for downloading PromptMan!
Compare cloud and local LLMs side by side, with token, cost and quality tracking.

HOW TO RUN:
-----------
1. Double-click 'promptman.exe' to launch the desktop application.
2. If Windows SmartScreen displays "Windows protected your PC":
   Click 'More info' -> Click 'Run anyway'.
   (PromptMan is open-source software built directly from verified source).

COMMAND LINE CLI:
-----------------
'promptman-cli.exe' is also included for command-line scripting and CI pipelines.
Example usage:
  promptman-cli.exe models
  promptman-cli.exe --help

FEATURES:
---------
- Side-by-side model comparison (OpenAI, Anthropic, Gemini, Groq, Mistral, Ollama, etc.)
- Real-time token counting, latency and cost tracking
- Offline & local-first: Ollama and LM Studio support
- Zero telemetry, API keys saved securely in Windows Credential Manager

OFFICIAL REPOSITORY:
--------------------
https://github.com/TIRK3Y/promptman
========================================================================
"@

Set-Content -Path (Join-Path $PackageDir "README.txt") -Value $ReadmeContent -Encoding UTF8

Write-Host "[3/4] Creating ZIP archive..." -ForegroundColor Yellow
if (Test-Path $ZipFile) {
    Remove-Item -Force $ZipFile
}
Compress-Archive -Path "$PackageDir\*" -DestinationPath $ZipFile -CompressionLevel Optimal

Write-Host "[4/4] Generating SHA256 checksums..." -ForegroundColor Yellow
$Hash = (Get-FileHash -Path $ZipFile -Algorithm SHA256).Hash.ToLower()
$ChecksumLine = "$Hash  $PackageName.zip"
Set-Content -Path (Join-Path $DistDir "SHA256SUMS-windows.txt") -Value $ChecksumLine -Encoding UTF8

Write-Host ""
Write-Host "========================================" -ForegroundColor Green
Write-Host " Packaging Completed Successfully!" -ForegroundColor Green
Write-Host " Output File: $ZipFile" -ForegroundColor Green
Write-Host " File Size:   $("{0:N2} MB" -f ((Get-Item $ZipFile).Length / 1MB))" -ForegroundColor Green
Write-Host " SHA256:      $Hash" -ForegroundColor Green
Write-Host "========================================" -ForegroundColor Green
