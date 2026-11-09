[CmdletBinding()]
$ErrorActionPreference = "Stop"

$workspaceRoot = Split-Path -Parent $PSScriptRoot
$logDirectory = Join-Path $env:TEMP "Faustus"
$stdoutLog = Join-Path $logDirectory "market-service.stdout.log"
$stderrLog = Join-Path $logDirectory "market-service.stderr.log"
$userAgent = "Faustus/0.1.0"
$quotedUserAgent = '"' + $userAgent + '"'
$serviceArguments = "run -p market-service -- serve --user-agent $quotedUserAgent"

New-Item -ItemType Directory -Path $logDirectory -Force | Out-Null

$existingListener = Get-NetTCPConnection -LocalPort 8787 -State Listen -ErrorAction SilentlyContinue
if ($existingListener) {
    throw "Port 8787 is already in use. Stop the existing service before using this launcher."
}

$serviceProcess = Start-Process `
    -FilePath "cargo" `
    -ArgumentList $serviceArguments `
    -WorkingDirectory $workspaceRoot `
    -WindowStyle Hidden `
    -RedirectStandardOutput $stdoutLog `
    -RedirectStandardError $stderrLog `
    -PassThru

try {
    $healthUri = "http://127.0.0.1:8787/health"
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds(90)
    $serviceReady = $false

    while ([DateTimeOffset]::UtcNow -lt $deadline) {
        $serviceProcess.Refresh()
        if ($serviceProcess.HasExited) {
            $errorOutput = if (Test-Path $stderrLog) {
                Get-Content -Path $stderrLog -Tail 30 | Out-String
            } else {
                "No service error log was created."
            }
            throw "The market service exited before becoming healthy.`n$errorOutput"
        }

        try {
            $null = Invoke-RestMethod -Uri $healthUri -TimeoutSec 2
            $serviceReady = $true
            break
        } catch {
            Start-Sleep -Milliseconds 500
        }
    }

    if (-not $serviceReady) {
        throw "The market service did not become healthy within 90 seconds. See $stderrLog"
    }

    Write-Host "Market service is ready at http://127.0.0.1:8787"
    Write-Host "Service logs: $logDirectory"
    Push-Location $workspaceRoot
    try {
        & cargo run -p market-ui --bin faustus-desktop
        if ($LASTEXITCODE -ne 0) {
            throw "The GPUI desktop app exited with code $LASTEXITCODE."
        }
    } finally {
        Pop-Location
    }
} finally {
    $serviceProcess.Refresh()
    if (-not $serviceProcess.HasExited) {
        & taskkill.exe /PID $serviceProcess.Id /T /F *> $null
    }
}
