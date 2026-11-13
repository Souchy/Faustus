[CmdletBinding()]
param(
    [switch]$Web,
    [switch]$Deprecated,
    [string]$Database = $env:FAUST_DATABASE,
    [string]$EconomyDatabase = $env:FAUST_ECONOMY_DATABASE
)

$ErrorActionPreference = "Stop"

$workspaceRoot = Split-Path -Parent $PSScriptRoot
$frontendRoot = Join-Path $workspaceRoot "crates\market-ui"
$logDirectory = Join-Path $env:TEMP "Faust"
$stdoutLog = Join-Path $logDirectory "market-service.stdout.log"
$stderrLog = Join-Path $logDirectory "market-service.stderr.log"
$userAgent = "Faust/0.1.0"
$quotedUserAgent = '"' + $userAgent + '"'
$serviceArguments = "serve --user-agent $quotedUserAgent"
if (-not $Database) {
    $Database = [Environment]::GetEnvironmentVariable("FAUST_DATABASE", "User")
}
if ($Database) {
    $serviceArguments += ' --database "' + $Database + '"'
}
if ($EconomyDatabase) {
    $serviceArguments += ' --economy-database "' + $EconomyDatabase + '"'
}

if ($Web -and $Deprecated) {
    throw "Choose either -Web or -Deprecated, not both."
}
if (-not $Deprecated -and -not (Test-Path (Join-Path $frontendRoot "node_modules"))) {
    throw "Install frontend dependencies first: cd crates\market-ui; npm ci"
}

New-Item -ItemType Directory -Path $logDirectory -Force | Out-Null

$existingListener = Get-NetTCPConnection -LocalPort 8787 -State Listen -ErrorAction SilentlyContinue
if ($existingListener) {
    throw "Port 8787 is already in use. Stop the existing service before using this launcher."
}

Push-Location $workspaceRoot
try {
    & cargo build -p market-service
    if ($LASTEXITCODE -ne 0) {
        throw "The market service failed to build."
    }
    $metadata = & cargo metadata --format-version 1 --no-deps
    if ($LASTEXITCODE -ne 0) {
        throw "Unable to locate the Cargo build directory."
    }
    $targetDirectory = ($metadata | ConvertFrom-Json).target_directory
} finally {
    Pop-Location
}

$serviceExecutable = Join-Path $targetDirectory "debug\market-service.exe"
$serviceProcess = Start-Process `
    -FilePath $serviceExecutable `
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
    $launchDirectory = if ($Deprecated) { $workspaceRoot } else { $frontendRoot }
    Push-Location $launchDirectory
    try {
        if ($Deprecated) {
            & cargo run -p market-ui-deprecated --bin faust-desktop-deprecated
        } elseif ($Web) {
            & npm run dev
        } else {
            & npm run tauri -- dev
        }
        if ($LASTEXITCODE -ne 0) {
            throw "The frontend exited with code $LASTEXITCODE."
        }
    } finally {
        Pop-Location
    }
} finally {
    $serviceProcess.Refresh()
    if (-not $serviceProcess.HasExited) {
        Stop-Process -Id $serviceProcess.Id
    }
}
