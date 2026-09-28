param(
    [ValidateSet("debug", "release")]
    [string]$Profile = "debug"
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$targetProfile = if ($Profile -eq "release") { "release" } else { "debug" }
$cargoProfile = if ($Profile -eq "release") { @("--release") } else { @() }
$runtimeExe = Join-Path $repo "target\$targetProfile\lab-runtime.exe"
$workbenchExe = Join-Path $repo "target\$targetProfile\lab-workbench.exe"
$temporary = Join-Path ([IO.Path]::GetTempPath()) ("lab-workbench-m14-4-" + [Guid]::NewGuid().ToString("N"))
$runtime = $null
$workbench = $null

function Start-Workbench([string]$Workspace, [string]$ResultVariable, [string]$ResultPath, [int]$Port) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $workbenchExe
    $info.UseShellExecute = $false
    $info.Arguments = "--connect 127.0.0.1:$Port --workspace `"$Workspace`""
    $info.Environment[$ResultVariable] = $ResultPath
    return [Diagnostics.Process]::Start($info)
}

function Wait-File([string]$Path, [int]$Seconds, [Diagnostics.Process]$Process) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    $windowObserved = $false
    $maximumWorkingSet = 0L
    while ([DateTime]::UtcNow -lt $deadline) {
        if (Test-Path -LiteralPath $Path) {
            return [pscustomobject]@{
                WindowObserved = $windowObserved
                MaximumWorkingSetBytes = $maximumWorkingSet
            }
        }
        if ($Process.HasExited) {
            throw "Workbench exited before producing $Path (exit $($Process.ExitCode))"
        }
        $Process.Refresh()
        $windowObserved = $windowObserved -or ($Process.MainWindowHandle -ne 0)
        $maximumWorkingSet = [Math]::Max($maximumWorkingSet, $Process.WorkingSet64)
        Start-Sleep -Milliseconds 50
    }
    throw "Timed out waiting for $Path"
}

try {
    New-Item -ItemType Directory -Path $temporary | Out-Null
    & cargo build -p lab-runtime -p lab-workbench --locked @cargoProfile
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    $runtimeInfo = [Diagnostics.ProcessStartInfo]::new()
    $runtimeInfo.FileName = $runtimeExe
    $runtimeInfo.UseShellExecute = $false
    $runtimeInfo.RedirectStandardOutput = $true
    $runtimeInfo.RedirectStandardError = $true
    $runtimeInfo.Arguments = "--serve --profile virtual-demo --port 0"
    $runtime = [Diagnostics.Process]::Start($runtimeInfo)
    $readyTask = $runtime.StandardOutput.ReadLineAsync()
    if (-not $readyTask.Wait([TimeSpan]::FromSeconds(5))) {
        throw "Runtime readiness deadline expired"
    }
    $readiness = $readyTask.Result | ConvertFrom-Json
    if (-not $readiness.port) { throw "Runtime readiness omitted TCP port" }

    $firstWorkspace = Join-Path $temporary "workspace"
    $smokeResult = Join-Path $temporary "gui-smoke.json"
    $workbench = Start-Workbench $firstWorkspace "LAB_WORKBENCH_GUI_SMOKE_RESULT" $smokeResult $readiness.port
    $firstMetrics = Wait-File $smokeResult 35 $workbench
    $result = Get-Content -LiteralPath $smokeResult -Raw | ConvertFrom-Json
    if ($result.status -ne "pass") { throw "GUI smoke reported $($result.status): $($result.reason)" }
    if (-not $firstMetrics.WindowObserved) { throw "No native Workbench window was observed" }
    if (-not $workbench.WaitForExit(5000)) { throw "Workbench close was not finite" }
    $workbench = $null
    if ($runtime.HasExited) { throw "Runtime exited when the Workbench closed" }

    $killReady = Join-Path $temporary "kill-ready.json"
    $workbench = Start-Workbench $firstWorkspace "LAB_WORKBENCH_GUI_KILL_READY" $killReady $readiness.port
    $killMetrics = Wait-File $killReady 35 $workbench
    if (-not $killMetrics.WindowObserved) { throw "No native kill-probe window was observed" }
    $workbench.Refresh()
    $idleCpuStart = $workbench.TotalProcessorTime.TotalMilliseconds
    $idleMemoryStart = $workbench.WorkingSet64
    Start-Sleep -Seconds 2
    $workbench.Refresh()
    $idleCpuMilliseconds = $workbench.TotalProcessorTime.TotalMilliseconds - $idleCpuStart
    $idleMemoryEnd = $workbench.WorkingSet64
    $workbench.Kill()
    $workbench.WaitForExit()
    $workbench = $null
    if ($runtime.HasExited) { throw "Runtime exited when the Workbench was forcibly terminated" }

    [pscustomobject]@{
        status = "pass"
        renderer = $result.renderer
        runtime_pid = $runtime.Id
        runtime_port = $readiness.port
        native_window_observed = $firstMetrics.WindowObserved
        minimize_restore_observed = $result.minimize_restore_observed
        maximum_working_set_bytes = $firstMetrics.MaximumWorkingSetBytes
        idle_cpu_milliseconds_over_2s = [Math]::Round($idleCpuMilliseconds, 1)
        idle_working_set_start_bytes = $idleMemoryStart
        idle_working_set_end_bytes = $idleMemoryEnd
        initial_fresh = $true
        stale_after_disconnect = $true
        fresh_after_reattach = $true
        live_signal_points = $result.live_signal_points
        reference_visible = $true
        runtime_alive_after_clean_close = (-not $runtime.HasExited)
        runtime_alive_after_forced_termination = (-not $runtime.HasExited)
    } | ConvertTo-Json -Compress
}
finally {
    if ($null -ne $workbench -and -not $workbench.HasExited) {
        $workbench.Kill()
        $workbench.WaitForExit()
    }
    if ($null -ne $runtime -and -not $runtime.HasExited) {
        $runtime.Kill()
        $runtime.WaitForExit()
    }
    if (Test-Path -LiteralPath $temporary) {
        Remove-Item -LiteralPath $temporary -Recurse -Force
    }
}
