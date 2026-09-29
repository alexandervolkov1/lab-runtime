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
$temporary = Join-Path ([IO.Path]::GetTempPath()) ("lab-workbench-m14-5-" + [Guid]::NewGuid().ToString("N"))
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

function Convert-StableJson($Value) {
    return ConvertTo-Json -InputObject $Value -Compress -Depth 20
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
    $recorderDatabase = Join-Path $temporary "runtime-recorder.sqlite"
    $runtimeInfo.Arguments = "--serve --profile virtual-demo --port 0 --record-db `"$recorderDatabase`" --record-policy best-effort"
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
    foreach ($required in @(
        "confirmation_observed",
        "mutation_accepted_observed",
        "mutation_completed_observed",
        "authoritative_refresh_observed",
        "stale_controls_disabled",
        "controls_reenabled_after_fresh"
    )) {
        if (-not $result.$required) { throw "GUI operator smoke did not prove $required" }
    }
    if (-not $firstMetrics.WindowObserved) { throw "No native Workbench window was observed" }
    if (-not $workbench.WaitForExit(5000)) { throw "Workbench close was not finite" }
    $workbench = $null
    if ($runtime.HasExited) { throw "Runtime exited when the Workbench closed" }

    $killReady = Join-Path $temporary "kill-ready.json"
    $workbench = Start-Workbench $firstWorkspace "LAB_WORKBENCH_GUI_KILL_READY" $killReady $readiness.port
    $killMetrics = Wait-File $killReady 35 $workbench
    $preKill = Get-Content -LiteralPath $killReady -Raw | ConvertFrom-Json
    if ($preKill.status -ne "ready_for_forced_termination") {
        throw "Kill-probe did not obtain authoritative continuity evidence: $($preKill.reason)"
    }
    if ($null -eq $preKill.controller) { throw "Controller was not observed before Workbench termination" }
    if ($null -eq $preKill.recorder) { throw "Recorder was not observed before Workbench termination" }
    if (-not $killMetrics.WindowObserved) { throw "No native kill-probe window was observed" }
    $workbench.Refresh()
    $idleCpuStart = $workbench.TotalProcessorTime.TotalMilliseconds
    $idleMemoryStart = $workbench.WorkingSet64
    Start-Sleep -Seconds 2
    $workbench.Refresh()
    $idleCpuMilliseconds = $workbench.TotalProcessorTime.TotalMilliseconds - $idleCpuStart
    $idleMemoryEnd = $workbench.WorkingSet64
    $workbench.Kill()
    if (-not $workbench.WaitForExit(5000)) { throw "Forced Workbench termination was not finite" }
    $workbench = $null
    if ($runtime.HasExited) { throw "Runtime exited when the Workbench was forcibly terminated" }

    # Reusing the same workspace is the executable ownership/journal oracle: the
    # killed process must release the mutex, and its durable recovery file must
    # remain parseable enough for a full authoritative rebuild and mutation cycle.
    $restartResult = Join-Path $temporary "post-kill-gui-smoke.json"
    $workbench = Start-Workbench $firstWorkspace "LAB_WORKBENCH_GUI_SMOKE_RESULT" $restartResult $readiness.port
    $restartMetrics = Wait-File $restartResult 35 $workbench
    $postKill = Get-Content -LiteralPath $restartResult -Raw | ConvertFrom-Json
    if ($postKill.status -ne "pass") { throw "Post-kill GUI restart reported $($postKill.status): $($postKill.reason)" }
    if (-not $restartMetrics.WindowObserved) { throw "No post-kill native Workbench window was observed" }
    if ($null -eq $postKill.controller) { throw "Controller was not observed after Workbench termination" }
    if ($null -eq $postKill.recorder) { throw "Recorder was not observed after Workbench termination" }
    $controllerContinuity =
        ($preKill.controller.controller -eq $postKill.controller.controller) -and
        ((Convert-StableJson $preKill.controller.state) -eq (Convert-StableJson $postKill.controller.state)) -and
        ((Convert-StableJson $preKill.controller.revision) -eq (Convert-StableJson $postKill.controller.revision))
    if (-not $controllerContinuity) {
        throw "Authoritative controller identity/state/revision changed across forced Workbench termination"
    }
    $recorderContinuity =
        ((Convert-StableJson $preKill.recorder.state) -eq (Convert-StableJson $postKill.recorder.state)) -and
        ((Convert-StableJson $preKill.recorder.active_run) -eq (Convert-StableJson $postKill.recorder.active_run)) -and
        ((Convert-StableJson $preKill.recorder.run_id) -eq (Convert-StableJson $postKill.recorder.run_id))
    if (-not $recorderContinuity) {
        throw "Authoritative Recorder state/run identity changed across forced Workbench termination"
    }
    if ($postKill.original_reference_revision -ne $result.final_reference_revision) {
        throw "Runtime reference revision did not survive forced Workbench termination"
    }
    if ([double]$postKill.desired_reference_target -le [double]$result.final_reference_target) {
        throw "Post-kill Runtime mutation did not advance from the retained authoritative target"
    }
    if (-not $workbench.WaitForExit(5000)) { throw "Post-kill Workbench close was not finite" }
    $workbench = $null
    if ($runtime.HasExited) { throw "Runtime exited during post-kill Workbench recovery" }

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
        confirmation_observed = $result.confirmation_observed
        mutation_accepted_observed = $result.mutation_accepted_observed
        mutation_completed_observed = $result.mutation_completed_observed
        authoritative_refresh_observed = $result.authoritative_refresh_observed
        stale_controls_disabled = $result.stale_controls_disabled
        controls_reenabled_after_fresh = $result.controls_reenabled_after_fresh
        runtime_alive_after_clean_close = (-not $runtime.HasExited)
        runtime_alive_after_forced_termination = (-not $runtime.HasExited)
        workspace_reacquired_after_forced_termination = $true
        journal_usable_after_forced_termination = $postKill.mutation_completed_observed
        runtime_domain_fresh_after_forced_termination = ($postKill.freshness -eq "Fresh")
        controller_observed_before_kill = ($null -ne $preKill.controller)
        controller_observed_after_kill = ($null -ne $postKill.controller)
        controller_continuity_proven = $controllerContinuity
        controller_before_kill = $preKill.controller
        controller_after_kill = $postKill.controller
        recorder_observed_before_kill = ($null -ne $preKill.recorder)
        recorder_observed_after_kill = ($null -ne $postKill.recorder)
        recorder_continuity_proven = $recorderContinuity
        recorder_before_kill = $preKill.recorder
        recorder_after_kill = $postKill.recorder
        authoritative_reference_continuity_after_forced_termination = $true
    } | ConvertTo-Json -Compress
}
finally {
    if ($null -ne $workbench -and -not $workbench.HasExited) {
        $workbench.Kill()
        [void]$workbench.WaitForExit(5000)
    }
    if ($null -ne $runtime -and -not $runtime.HasExited) {
        $runtime.Kill()
        [void]$runtime.WaitForExit(5000)
    }
    if (Test-Path -LiteralPath $temporary) {
        Remove-Item -LiteralPath $temporary -Recurse -Force
    }
}
