param([string]$Executable = 'target/release/sound-lock.exe')
# Run an optimized ui-capture build on an interactive Windows desktop.
$ErrorActionPreference='Stop'
New-Item -ItemType Directory -Force .local | Out-Null
$renderExecutable=(Resolve-Path -LiteralPath $Executable).Path
$saved=@{}
foreach($name in @('SOUND_LOCK_RENDER_ACTIVITY','SOUND_LOCK_RENDER_SECONDS','SOUND_LOCK_RENDER_REPORT','SOUND_LOCK_CAPTURE_VIEW','SOUND_LOCK_CAPTURE_TO','SOUND_LOCK_UI_SCALE','SOUND_LOCK_TRAY_SMOKE_REPORT')) {$saved[$name]=[Environment]::GetEnvironmentVariable($name,'Process');[Environment]::SetEnvironmentVariable($name,$null,'Process')}
try {
    foreach($scenario in @('settings-idle','hidden-idle','popup-idle','popup-meter')) {
        $env:SOUND_LOCK_RENDER_SECONDS='10'
        $env:SOUND_LOCK_RENDER_REPORT=Join-Path (Get-Location) ".local/cpu-$scenario-frames.json"
        $arguments=switch($scenario) {'settings-idle' {@('--preview','--settings')} 'hidden-idle' {@('--preview','--startup')} default {@('--preview')}}
        $env:SOUND_LOCK_RENDER_ACTIVITY=if($scenario -eq 'popup-meter'){'1'}else{$null}
        $env:SOUND_LOCK_CAPTURE_VIEW=if($scenario -eq 'popup-meter'){'popup'}else{$null}
        $renderProcess=Start-Process -FilePath $renderExecutable -ArgumentList $arguments -WindowStyle Hidden -PassThru -RedirectStandardError ".local/cpu-$scenario.log"
        Start-Sleep -Seconds 3
        $renderProcess.Refresh()
        $initialCpu=$renderProcess.TotalProcessorTime.TotalSeconds
        $measureClock=[Diagnostics.Stopwatch]::StartNew()
        Start-Sleep -Seconds 5
        $renderProcess.Refresh()
        $measureClock.Stop()
        $corePercent=100*($renderProcess.TotalProcessorTime.TotalSeconds-$initialCpu)/$measureClock.Elapsed.TotalSeconds
        $report=[pscustomobject]@{scenario=$scenario;pid=$renderProcess.Id;path=$renderProcess.Path;one_core_percent=$corePercent;machine_percent=$corePercent/[Environment]::ProcessorCount;logical_cpus=[Environment]::ProcessorCount;sample_seconds=$measureClock.Elapsed.TotalSeconds;graphics_modules=@($renderProcess.Modules | Where-Object {$_.ModuleName -match 'opengl|nvoglv|dxgi|d3d'} | Select-Object -ExpandProperty ModuleName)}
        $report | ConvertTo-Json | Set-Content ".local/cpu-$scenario-cpu.json"
        if(-not $renderProcess.WaitForExit(5000)) {Stop-Process -Id $renderProcess.Id;throw "Render test timed out: $scenario"}
        if($renderProcess.ExitCode -ne 0) {Get-Content ".local/cpu-$scenario.log";throw "Render test failed: $scenario"}
        $report | ConvertTo-Json
        Get-Content $env:SOUND_LOCK_RENDER_REPORT
    }
}finally {foreach($name in $saved.Keys){[Environment]::SetEnvironmentVariable($name,$saved[$name],'Process')}}
