[CmdletBinding()]
param([ValidateRange(1, 10)][int]$Repeat = 3)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$bb = (Get-Command bb -ErrorAction Stop).Source
$temporary = Join-Path ([IO.Path]::GetTempPath()) ('lab-m17-5-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $temporary | Out-Null
$runtime = $null
$workbench = $null

function Start-Fixture([string]$Executable, [string[]]$Arguments, [string]$Name) {
    $info = [Diagnostics.ProcessStartInfo]::new($Executable)
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $info.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::Start($info)
    # Drain stderr asynchronously so diagnostics cannot block fixture progress.
    $stderr = $process.StandardError.ReadToEndAsync()
    return @{ Process = $process; Stderr = $stderr; Name = $Name }
}

function Stop-Fixture($Fixture) {
    if ($null -eq $Fixture) { return }
    if (-not $Fixture.Process.HasExited) { $Fixture.Process.Kill($true) }
    if (-not $Fixture.Process.WaitForExit(5000)) { throw "Finite cleanup failed: $($Fixture.Name)" }
    [IO.File]::WriteAllText((Join-Path $temporary "$($Fixture.Name).stderr.txt"), $Fixture.Stderr.GetAwaiter().GetResult())
    $Fixture.Process.Dispose()
}

function Read-Ready($Fixture) {
    $line = $Fixture.Process.StandardOutput.ReadLineAsync()
    if (-not $line.Wait(10000)) { throw "Readiness deadline: $($Fixture.Name)" }
    if ([string]::IsNullOrEmpty($line.Result)) { throw "No readiness: $($Fixture.Name)" }
    return $line.Result | ConvertFrom-Json
}

function Invoke-Smoke([string]$Script, [int]$Port, [int]$Run) {
    $fixture = Start-Fixture $bb @((Join-Path $PSScriptRoot $Script), "$Port") "$Script-$Run"
    try {
        $stdout = $fixture.Process.StandardOutput.ReadToEndAsync()
        if (-not $fixture.Process.WaitForExit(15000)) { throw "Babashka deadline: $Script" }
        if ($fixture.Process.ExitCode -ne 0) { throw "Babashka failed: $($fixture.Stderr.GetAwaiter().GetResult())" }
        $summary = $stdout.GetAwaiter().GetResult() | ConvertFrom-Json
        if ($summary.status -ne 'PASS') { throw "Babashka omitted PASS: $Script" }
        $summary | ConvertTo-Json -Compress
    }
    finally { Stop-Fixture $fixture }
}

try {
    $runtime = Start-Fixture (Join-Path $repo 'target/debug/lab-runtime.exe') @(
        '--serve', '--profile', 'virtual-demo', '--port', '0'
    ) 'runtime'
    $ready = Read-Ready $runtime
    if ($ready.port -lt 1) { throw 'Runtime omitted bound port' }
    $workbench = Start-Fixture (Join-Path $repo 'target/debug/lab-workbench.exe') @(
        '--connect', "127.0.0.1:$($ready.port)", '--workspace', (Join-Path $temporary 'workspace'),
        '--workbench-listen', '127.0.0.1:0'
    ) 'workbench'
    $endpoint = [Net.IPEndPoint]::Parse((Read-Ready $workbench).workbench_endpoint)
    if ($endpoint.Address.ToString() -ne '127.0.0.1' -or $endpoint.Port -lt 1) { throw 'Unexpected Workbench address' }
    foreach ($run in 1..$Repeat) {
        Invoke-Smoke 'runtime.clj' $ready.port $run
        Invoke-Smoke 'workbench.clj' $endpoint.Port $run
        if ($runtime.Process.HasExited -or $workbench.Process.HasExited) { throw 'Caller exit stopped a fixture' }
    }
    Stop-Fixture $workbench
    $workbench = $null
    if ($runtime.Process.HasExited) { throw 'Workbench termination stopped Runtime' }
    Write-Output "PASS: $Repeat direct Runtime + $Repeat Workbench Babashka runs; finite cleanup; logs: $temporary"
}
finally {
    try { Stop-Fixture $workbench }
    finally { Stop-Fixture $runtime }
}
