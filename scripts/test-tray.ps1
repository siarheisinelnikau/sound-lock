# Run from an interactive Windows desktop. All application data is simulated.
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$previousLocation = Get-Location
$previousReport = $env:SOUND_LOCK_TRAY_SMOKE_REPORT
$previousCapture = $env:SOUND_LOCK_CAPTURE_TO
$previousView = $env:SOUND_LOCK_CAPTURE_VIEW
try {
    Set-Location -LiteralPath $projectRoot
    Remove-Item Env:SOUND_LOCK_CAPTURE_TO -ErrorAction SilentlyContinue
    Remove-Item Env:SOUND_LOCK_CAPTURE_VIEW -ErrorAction SilentlyContinue
    cargo build --release --locked --features tray-smoke-test --target-dir .local/tray-test
    if ($LASTEXITCODE -ne 0) { throw 'Could not build the tray test' }
    $testExecutable = (Resolve-Path .local/tray-test/release/sound-lock.exe).Path
    foreach ($startMode in @('normal', 'quiet')) {
        $env:SOUND_LOCK_TRAY_SMOKE_REPORT = Join-Path $projectRoot ".local/tray-test-$startMode.json"
        $logPath = Join-Path $projectRoot ".local/tray-test-$startMode.log"
        $testArguments = if ($startMode -eq 'quiet') { @('--preview','--startup') } else { @('--preview') }
        $testProcess = Start-Process -FilePath $testExecutable -ArgumentList $testArguments -WindowStyle Hidden -PassThru -RedirectStandardError $logPath
        if (-not $testProcess.WaitForExit(30000)) {
            Stop-Process -Id $testProcess.Id
            throw "Tray check timed out: $startMode"
        }
        Get-Content -LiteralPath $logPath
        if ($testProcess.ExitCode -ne 0) { throw "Tray check failed: $startMode" }
        $report = Get-Content -LiteralPath $env:SOUND_LOCK_TRAY_SMOKE_REPORT -Raw | ConvertFrom-Json
        if ($report.status -ne 'passed') { throw "Invalid tray report: $startMode" }
        Write-Output "PASS $startMode tray lifecycle"
    }
} finally {
    $env:SOUND_LOCK_TRAY_SMOKE_REPORT = $previousReport
    $env:SOUND_LOCK_CAPTURE_TO = $previousCapture
    $env:SOUND_LOCK_CAPTURE_VIEW = $previousView
    Set-Location -LiteralPath $previousLocation
}
