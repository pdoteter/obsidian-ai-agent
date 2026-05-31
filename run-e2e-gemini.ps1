# Obsidian AI Agent E2E Playwright Test Runner (Google Gemini Provider - Windows PowerShell)
$ErrorActionPreference = "Stop"

$testVaultDir = Join-Path $PSScriptRoot "test-vault"
$assetsDir = Join-Path $testVaultDir "assets"
$obsidianConfigDir = Join-Path $testVaultDir ".obsidian"

Write-Host "[E2E] Preparing clean test vault environment..." -ForegroundColor Cyan

# 1. Clean up and recreate isolated test vault
if (Test-Path $testVaultDir) {
    Remove-Item -Path $testVaultDir -Recurse -Force
}
New-Item -Path $testVaultDir -ItemType Directory | Out-Null
New-Item -Path $assetsDir -ItemType Directory | Out-Null
New-Item -Path $obsidianConfigDir -ItemType Directory | Out-Null

# Create Obsidian daily-notes.json settings
$obsidianSettings = @{
    folder = ""
    format = "YYYY-MM-DD"
    template = ""
} | ConvertTo-Json -Compress
Set-Content -Path (Join-Path $obsidianConfigDir "daily-notes.json") -Value $obsidianSettings

# Create local test config.yaml (pointing to test-vault using gemini provider)
$testConfig = @"
vault_path: "$($testVaultDir.Replace('\', '/'))"
webui:
  enabled: true
  port: 3000
git:
  sync_enabled: false
ai:
  provider: "gemini"
  whisper_model: "whisper-1"
  classify_model: "gemini-2.5-flash"
  transcription:
    provider: "openai"
"@
Set-Content -Path (Join-Path $testVaultDir "config.yaml") -Value $testConfig

# Load existing environment variables from .env if present
if (Test-Path (Join-Path $PSScriptRoot ".env")) {
    Write-Host "[E2E] Loading environment secrets from .env..." -ForegroundColor Gray
    Get-Content (Join-Path $PSScriptRoot ".env") | Where-Object { $_ -match '=' -and $_ -notmatch '^#' } | ForEach-Object {
        $name, $value = $_.Split('=', 2)
        [System.Environment]::SetEnvironmentVariable($name.Trim(), $value.Trim())
    }
}

# Ensure critical API keys are present (since E2E runs real APIs)
if (-not $env:GEMINI_API_KEY -or -not $env:TELOXIDE_TOKEN) {
    Write-Host "[E2E ERROR] Real API keys (GEMINI_API_KEY, TELOXIDE_TOKEN) must be set in your environment or .env file to run these live E2E tests!" -ForegroundColor Red
    Exit 1
}

# Overrides for testing
$env:CONFIG_PATH = Join-Path $testVaultDir "config.yaml"
$env:WEBUI_AUTH_TOKEN = "test_token"

Write-Host "[E2E] Compiling Obsidian AI Agent in debug mode..." -ForegroundColor Cyan
cargo build

Write-Host "[E2E] Launching Obsidian AI Agent bot in background (using Gemini)..." -ForegroundColor Cyan
$botBin = Join-Path $PSScriptRoot "target\debug\obsidian-ai-agent.exe"
$botLog = Join-Path $PSScriptRoot "bot-e2e.log"
$botErr = Join-Path $PSScriptRoot "bot-e2e-err.log"

$botProc = Start-Process -FilePath $botBin -PassThru -NoNewWindow -RedirectStandardOutput $botLog -RedirectStandardError $botErr

Write-Host "[E2E] Waiting for server to boot and bind to port 3000..." -ForegroundColor Gray
Start-Sleep -Seconds 6

# Check if bot is still running
if ($botProc.HasExited) {
    Write-Host "[E2E ERROR] Rust bot server exited unexpectedly during startup!" -ForegroundColor Red
    if (Test-Path $botErr) {
        Write-Host "=== ERROR LOG ===" -ForegroundColor Yellow
        Get-Content $botErr -Tail 20
    }
    Exit 1
}

Write-Host "[E2E] Installing Node dependencies & browser binaries for Playwright..." -ForegroundColor Cyan
Push-Location (Join-Path $PSScriptRoot "e2e-tests")
npm install
npx playwright install chromium

Write-Host "[E2E] Executing Playwright End-to-End Test Suite against Gemini..." -ForegroundColor Green
$testError = $null
try {
    npx playwright test
    if ($LASTEXITCODE -ne 0) {
        throw "Playwright tests exited with code $LASTEXITCODE"
    }
    Write-Host "[E2E SUCCESS] All integration test cases passed flawlessly!" -ForegroundColor Green
} catch {
    $testError = $_
    Write-Host "[E2E ERROR] Integration tests encountered failures!" -ForegroundColor Red
}

Pop-Location

# 5. Clean up background bot and temporary vault
Write-Host "[E2E] Cleaning up and shutting down background server..." -ForegroundColor Cyan
if ($botProc -and -not $botProc.HasExited) {
    $botProc.Kill()
    $botProc.WaitForExit()
}

# Remove temporary test vault and assets
if (Test-Path $testVaultDir) {
    # wait a bit for file handles to be released
    Start-Sleep -Seconds 2
    Remove-Item -Path $testVaultDir -Recurse -Force
}

# Clean up log files on success
if (-not $testError) {
    if (Test-Path $botLog) { Remove-Item $botLog -Force }
    if (Test-Path $botErr) { Remove-Item $botErr -Force }
}

if ($testError) {
    Write-Host "[E2E] Test Suite failed: $testError" -ForegroundColor Red
    Exit 1
} else {
    Write-Host "[E2E SUCCESS] E2E Playwright Run Completed Successfully with Gemini!" -ForegroundColor Green
    Exit 0
}
