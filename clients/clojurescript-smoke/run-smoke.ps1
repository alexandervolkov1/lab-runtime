[CmdletBinding()]
param(
    [ValidateRange(1, 65535)]
    [int]$OriginPort = 9000,
    [string]$RuntimeExe,
    [string]$BrowserExe,
    [ValidateRange(20, 120)]
    [int]$TimeoutSeconds = 60
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$compilerVersion = '1.12.145'
$origin = "http://127.0.0.1:$OriginPort"
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$sourceRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'src'))
$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$tempRoot = Join-Path $tempBase ("lab-runtime-m12-5-" + [guid]::NewGuid().ToString('N'))
$runtime = $null
$http = $null
$browser = $null
$runtimeOut = $null
$runtimeErr = $null
$httpErr = $null
$browserErr = $null

function Stop-GuardedProcess {
    param([Diagnostics.Process]$Process)
    if ($null -ne $Process -and -not $Process.HasExited) {
        $Process.Kill($true)
        $Process.WaitForExit(5000) | Out-Null
    }
}

function Wait-FileLine {
    param(
        [string]$Path,
        [Diagnostics.Process]$Process,
        [datetime]$Deadline
    )
    while ([datetime]::UtcNow -lt $Deadline) {
        if ($Process.HasExited) {
            throw "Process exited before readiness: $($Process.ExitCode)"
        }
        if (Test-Path -LiteralPath $Path) {
            $line = Get-Content -LiteralPath $Path -TotalCount 1 -ErrorAction SilentlyContinue
            if (-not [string]::IsNullOrWhiteSpace($line)) {
                return $line
            }
        }
        Start-Sleep -Milliseconds 50
    }
    throw "Timed out waiting for readiness output"
}

function Resolve-Browser {
    if (-not [string]::IsNullOrWhiteSpace($BrowserExe)) {
        return [IO.Path]::GetFullPath($BrowserExe)
    }
    $candidates = @(
        "$env:ProgramFiles\Google\Chrome\Application\chrome.exe",
        "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe",
        "$env:LOCALAPPDATA\Google\Chrome\Application\chrome.exe",
        "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe",
        "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe",
        "$env:LOCALAPPDATA\Microsoft\Edge\Application\msedge.exe"
    )
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate) {
            return [IO.Path]::GetFullPath($candidate)
        }
    }
    throw 'No installed Chrome or Edge browser was found.'
}

function Resolve-RuntimeCommit {
    param([string]$Root)

    $buildMetadata = Join-Path $Root 'BUILD.txt'
    if (Test-Path -LiteralPath $buildMetadata) {
        $commitLine = Get-Content -LiteralPath $buildMetadata |
            Where-Object { $_ -match '^git-commit=' } |
            Select-Object -First 1
        if ($commitLine -notmatch '^git-commit=([0-9a-f]{40})$') {
            throw "BUILD.txt does not contain a valid git-commit entry: $buildMetadata"
        }
        return $Matches[1]
    }

    $commit = (& git -C $Root rev-parse HEAD 2>$null)
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($commit)) {
        throw "Could not determine the Runtime commit from BUILD.txt or Git: $Root"
    }
    $commit = $commit.Trim()
    if ($commit -notmatch '^[0-9a-f]{40}$') {
        throw "Git returned an invalid Runtime commit: $commit"
    }
    return $commit
}

function Wait-DevToolsPort {
    param(
        [string]$Profile,
        [Diagnostics.Process]$Process,
        [datetime]$Deadline
    )
    $activePort = Join-Path $Profile 'DevToolsActivePort'
    while ([datetime]::UtcNow -lt $Deadline) {
        if ($Process.HasExited) {
            throw "Browser exited before DevTools readiness: $($Process.ExitCode)"
        }
        if (Test-Path -LiteralPath $activePort) {
            $lines = @(Get-Content -LiteralPath $activePort -ErrorAction SilentlyContinue)
            if ($lines.Count -ge 1 -and $lines[0] -match '^\d+$') {
                return [int]$lines[0]
            }
        }
        Start-Sleep -Milliseconds 50
    }
    throw 'Browser DevTools readiness deadline exceeded.'
}

function Invoke-DevToolsExpression {
    param(
        [Net.WebSockets.ClientWebSocket]$Socket,
        [int]$Id,
        [string]$Expression,
        [datetime]$Deadline
    )
    $command = @{id = $Id; method = 'Runtime.evaluate'; params = @{
        expression = $Expression
        returnByValue = $true
    }} | ConvertTo-Json -Compress -Depth 5
    $bytes = [Text.Encoding]::UTF8.GetBytes($command)
    $Socket.SendAsync(
        [ArraySegment[byte]]::new($bytes),
        [Net.WebSockets.WebSocketMessageType]::Text,
        $true,
        [Threading.CancellationToken]::None
    ).GetAwaiter().GetResult() | Out-Null

    while ([datetime]::UtcNow -lt $Deadline) {
        $buffer = [byte[]]::new(8192)
        $stream = [IO.MemoryStream]::new()
        do {
            $remaining = $Deadline - [datetime]::UtcNow
            if ($remaining -le [timespan]::Zero) {
                throw 'DevTools response deadline exceeded.'
            }
            $cancel = [Threading.CancellationTokenSource]::new($remaining)
            try {
                $received = $Socket.ReceiveAsync(
                    [ArraySegment[byte]]::new($buffer),
                    $cancel.Token
                ).GetAwaiter().GetResult()
            }
            finally {
                $cancel.Dispose()
            }
            $stream.Write($buffer, 0, $received.Count)
        } while (-not $received.EndOfMessage)
        $message = [Text.Encoding]::UTF8.GetString($stream.ToArray()) | ConvertFrom-Json
        if ($message.id -eq $Id) {
            if ($null -ne $message.result.PSObject.Properties['exceptionDetails']) {
                throw "DevTools expression failed: $($message.result.exceptionDetails.text)"
            }
            return $message.result.result.value
        }
    }
    throw 'DevTools response deadline exceeded.'
}

try {
    $probe = [Net.Sockets.TcpListener]::new(
        [Net.IPAddress]::Parse('127.0.0.1'),
        $OriginPort
    )
    try {
        $probe.Start()
    }
    finally {
        $probe.Stop()
    }

    New-Item -ItemType Directory -Path $tempRoot | Out-Null
    $outputDir = Join-Path $tempRoot 'closure-output'
    $mainJs = Join-Path $tempRoot 'main.js'
    New-Item -ItemType Directory -Path $outputDir | Out-Null

    $sourceEdn = $sourceRoot.Replace('\', '/')
    $deps = "{:paths [`"$sourceEdn`"] :deps {org.clojure/clojurescript {:mvn/version `"$compilerVersion`"}}}"
    Push-Location $tempRoot
    try {
        & clojure -Sdeps $deps -M -m cljs.main -co '{:browser-repl false}' -O simple -d $outputDir -o $mainJs -c lab-runtime-smoke.core
        if ($LASTEXITCODE -ne 0) {
            throw "ClojureScript compilation failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        Pop-Location
    }

    if ([string]::IsNullOrWhiteSpace($RuntimeExe)) {
        $RuntimeExe = Join-Path $repoRoot 'target\debug\lab-runtime.exe'
    }
    $RuntimeExe = [IO.Path]::GetFullPath($RuntimeExe)
    if (-not (Test-Path -LiteralPath $RuntimeExe)) {
        throw "Runtime executable not found: $RuntimeExe. Run cargo build -p lab-runtime --locked first."
    }

    $runtimeOut = Join-Path $tempRoot 'runtime.stdout.txt'
    $runtimeErr = Join-Path $tempRoot 'runtime.stderr.txt'
    $runtime = Start-Process -FilePath $RuntimeExe -ArgumentList @(
        '--serve', '--profile', 'virtual-demo', '--port', '0',
        '--ws-port', '0', '--ws-origin', $origin
    ) -PassThru -WindowStyle Hidden -RedirectStandardOutput $runtimeOut -RedirectStandardError $runtimeErr
    $readyLine = Wait-FileLine -Path $runtimeOut -Process $runtime -Deadline ([datetime]::UtcNow.AddSeconds(10))
    $readiness = $readyLine | ConvertFrom-Json
    if ($readiness.port -lt 1 -or $readiness.websocket.port -lt 1) {
        throw "Runtime readiness did not contain bound TCP/WS ports: $readyLine"
    }
    if ($readiness.websocket.path -ne '/application/v1' -or
        $readiness.websocket.subprotocol -ne 'lab-runtime.application.v1') {
        throw "Runtime readiness advertised an unexpected WebSocket contract: $readyLine"
    }
    $wsEndpoint = "ws://127.0.0.1:$($readiness.websocket.port)$($readiness.websocket.path)"

    $config = [ordered]@{
        wsUrl = $wsEndpoint
        subprotocol = 'lab-runtime.application.v1'
        stepTimeoutMs = 5000
        reattachTimeoutMs = 5000
        overallTimeoutMs = 30000
    } | ConvertTo-Json -Compress
    $html = @"
<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>M12.5 browser smoke</title></head>
<body>
<main id="app" data-status="starting">
<h1>M12.5 browser smoke</h1>
<pre id="result">starting</pre>
</main>
<script>window.LAB_RUNTIME_SMOKE_CONFIG=$config;</script>
<script src="/main.js"></script>
</body>
</html>
"@
    [IO.File]::WriteAllText((Join-Path $tempRoot 'index.html'), $html, [Text.UTF8Encoding]::new($false))

    $python = (Get-Command py -ErrorAction Stop).Source
    $httpOut = Join-Path $tempRoot 'http.stdout.txt'
    $httpErr = Join-Path $tempRoot 'http.stderr.txt'
    $http = Start-Process -FilePath $python -ArgumentList @(
        '-3', '-m', 'http.server', $OriginPort, '--bind', '127.0.0.1', '--directory', $tempRoot
    ) -PassThru -WindowStyle Hidden -RedirectStandardOutput $httpOut -RedirectStandardError $httpErr

    $httpDeadline = [datetime]::UtcNow.AddSeconds(10)
    $httpReady = $false
    while ([datetime]::UtcNow -lt $httpDeadline -and -not $httpReady) {
        if ($http.HasExited) {
            throw "HTTP server exited before readiness: $($http.ExitCode)"
        }
        try {
            $response = Invoke-WebRequest -Uri "$origin/" -UseBasicParsing -TimeoutSec 1
            $httpReady = $response.StatusCode -eq 200
        }
        catch {
            Start-Sleep -Milliseconds 50
        }
    }
    if (-not $httpReady) {
        throw 'HTTP server readiness deadline exceeded.'
    }

    $BrowserExe = Resolve-Browser
    $browserVersion = (Get-Item -LiteralPath $BrowserExe).VersionInfo.ProductVersion
    $browserProfile = Join-Path $tempRoot 'browser-profile'
    New-Item -ItemType Directory -Path $browserProfile | Out-Null
    $browserOut = Join-Path $tempRoot 'browser.stdout.txt'
    $browserErr = Join-Path $tempRoot 'browser.stderr.txt'
    $browser = Start-Process -FilePath $BrowserExe -ArgumentList @(
        '--headless=new', '--disable-gpu', '--disable-extensions', '--no-first-run',
        '--no-default-browser-check', '--disable-background-networking',
        "--user-data-dir=$browserProfile", '--remote-debugging-address=127.0.0.1',
        '--remote-debugging-port=0', "$origin/"
    ) -PassThru -WindowStyle Hidden -RedirectStandardOutput $browserOut -RedirectStandardError $browserErr
    $browserDeadline = [datetime]::UtcNow.AddSeconds($TimeoutSeconds)
    $debugPort = Wait-DevToolsPort -Profile $browserProfile -Process $browser -Deadline $browserDeadline
    $page = $null
    while ([datetime]::UtcNow -lt $browserDeadline -and $null -eq $page) {
        $targets = Invoke-RestMethod -Uri "http://127.0.0.1:$debugPort/json/list" -TimeoutSec 1
        $page = $targets | Where-Object { $_.type -eq 'page' -and $_.url -eq "$origin/" } | Select-Object -First 1
        if ($null -eq $page) {
            Start-Sleep -Milliseconds 50
        }
    }
    if ($null -eq $page) {
        throw 'Browser page target readiness deadline exceeded.'
    }
    $devTools = [Net.WebSockets.ClientWebSocket]::new()
    try {
        $devTools.ConnectAsync(
            [Uri]$page.webSocketDebuggerUrl,
            [Threading.CancellationToken]::None
        ).GetAwaiter().GetResult() | Out-Null
        $commandId = 0
        $status = 'starting'
        while ([datetime]::UtcNow -lt $browserDeadline -and $status -notin @('pass', 'fail')) {
            $commandId++
            $status = Invoke-DevToolsExpression -Socket $devTools -Id $commandId `
                -Expression 'document.getElementById("app")?.getAttribute("data-status")' `
                -Deadline $browserDeadline
            if ($status -notin @('pass', 'fail')) {
                Start-Sleep -Milliseconds 50
            }
        }
        $commandId++
        $summaryText = Invoke-DevToolsExpression -Socket $devTools -Id $commandId `
            -Expression 'document.getElementById("result")?.textContent' `
            -Deadline $browserDeadline
        if ($status -ne 'pass') {
            throw "Browser smoke did not report PASS. Status: $status Result: $summaryText"
        }
    }
    finally {
        $devTools.Dispose()
    }
    $summary = $summaryText | ConvertFrom-Json
    if ($summary.selected_subprotocol -ne 'lab-runtime.application.v1' -or
        -not $summary.replay_observed -or -not $summary.clean_browser_close) {
        throw "Browser summary omitted required acceptance facts: $summaryText"
    }

    $javaVersion = (& java -version 2>&1 | Select-Object -First 1).ToString()
    $runtimeCommit = Resolve-RuntimeCommit -Root $repoRoot
    $evidence = [ordered]@{
        status = 'PASS'
        runtime_commit = $runtimeCommit
        os = [Environment]::OSVersion.VersionString
        java = $javaVersion
        clojurescript = $compilerVersion
        browser = (Split-Path -Leaf $BrowserExe)
        browser_version = $browserVersion
        http_origin = $origin
        readiness = [ordered]@{
            tcp_port = $readiness.port
            websocket_port = $readiness.websocket.port
            websocket_path = $readiness.websocket.path
            websocket_subprotocol = $readiness.websocket.subprotocol
        }
        websocket_endpoint = $wsEndpoint
        result = $summary
    }
    $evidence | ConvertTo-Json -Depth 10
}
catch {
    $diagnostics = [ordered]@{}
    foreach ($entry in @(
        @('runtime_stdout', $runtimeOut),
        @('runtime_stderr', $runtimeErr),
        @('http_stderr', $httpErr),
        @('browser_stderr', $browserErr)
    )) {
        if ($null -ne $entry[1] -and (Test-Path -LiteralPath $entry[1])) {
            $diagnostics[$entry[0]] = Get-Content -LiteralPath $entry[1] -Raw
        }
    }
    Write-Warning ("M12.5 diagnostics: " + ($diagnostics | ConvertTo-Json -Compress))
    throw
}
finally {
    Stop-GuardedProcess $browser
    Stop-GuardedProcess $http
    Stop-GuardedProcess $runtime
    if (Test-Path -LiteralPath $tempRoot) {
        $resolvedTemp = [IO.Path]::GetFullPath($tempRoot)
        if (-not $resolvedTemp.StartsWith($tempBase, [StringComparison]::OrdinalIgnoreCase) -or
            -not ([IO.Path]::GetFileName($resolvedTemp)).StartsWith('lab-runtime-m12-5-', [StringComparison]::Ordinal)) {
            throw "Refusing to remove unexpected temporary path: $resolvedTemp"
        }
        Remove-Item -LiteralPath $resolvedTemp -Recurse -Force
    }
}
