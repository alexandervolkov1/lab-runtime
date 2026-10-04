[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^v[0-9]+\.[0-9]+\.[0-9]+-preview\.[1-9][0-9]*$')]
    [string]$PreviewVersion,
    [switch]$AllowDirty
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$packageName = "lab-runtime-$PreviewVersion-windows-x86_64"
$targetTriple = 'x86_64-pc-windows-msvc'
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$distRoot = [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot 'dist'))
$stageRoot = [System.IO.Path]::GetFullPath((Join-Path $distRoot $packageName))
$zipPath = [System.IO.Path]::GetFullPath((Join-Path $distRoot "$packageName.zip"))
$checksumPath = "$zipPath.sha256"

function Assert-Success([string]$operation) {
    if ($LASTEXITCODE -ne 0) {
        throw "$operation failed with exit code $LASTEXITCODE"
    }
}

function Assert-ChildPath([string]$path, [string]$parent) {
    $prefix = $parent.TrimEnd([System.IO.Path]::DirectorySeparatorChar) +
        [System.IO.Path]::DirectorySeparatorChar
    if (-not $path.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "refusing to modify path outside $parent`: $path"
    }
}

function Copy-ApprovedFile([string]$relativeSource, [string]$relativeDestination) {
    $source = Join-Path $repositoryRoot $relativeSource
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "required package input is missing: $relativeSource"
    }
    $destination = Join-Path $stageRoot $relativeDestination
    $destinationDirectory = Split-Path -Parent $destination
    [System.IO.Directory]::CreateDirectory($destinationDirectory) | Out-Null
    Copy-Item -LiteralPath $source -Destination $destination
}

function Get-MarkdownHeadingIds([string]$path) {
    $ids = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::OrdinalIgnoreCase
    )
    $occurrences = @{}
    $insideFence = $false
    foreach ($line in Get-Content -LiteralPath $path) {
        if ($line -match '^\s*(```|~~~)') {
            $insideFence = -not $insideFence
            continue
        }
        if ($insideFence -or $line -notmatch '^\s{0,3}#{1,6}\s+(?<heading>.+?)\s*#*\s*$') {
            continue
        }
        $heading = $Matches.heading
        $heading = [regex]::Replace($heading, '\[([^\]]+)\]\([^)]+\)', '$1')
        $heading = $heading.Replace('`', '')
        $id = $heading.ToLowerInvariant()
        $id = [regex]::Replace($id, '[^\p{L}\p{Nd}\s_-]', '')
        $id = [regex]::Replace($id.Trim(), '\s+', '-')
        if (-not $id) {
            continue
        }
        $count = if ($occurrences.ContainsKey($id)) { [int]$occurrences[$id] } else { 0 }
        $occurrences[$id] = $count + 1
        if ($count -gt 0) {
            $id = "$id-$count"
        }
        [void]$ids.Add($id)
    }
    return ,$ids
}

function Test-MarkdownQuality([string]$packageRoot) {
    $issues = [System.Collections.Generic.List[string]]::new()
    $markdownFiles = @(Get-ChildItem -LiteralPath $packageRoot -Recurse -Filter '*.md' -File |
        Where-Object {
            $relativePath = $_.FullName.Substring($packageRoot.Length + 1)
            $relativePath -eq 'README.md' -or
                $relativePath.StartsWith('docs\', [System.StringComparison]::OrdinalIgnoreCase) -or
                $relativePath.StartsWith(
                    'clients\',
                    [System.StringComparison]::OrdinalIgnoreCase
                )
        })
    foreach ($file in $markdownFiles) {
        $relativeFile = $file.FullName.Substring($packageRoot.Length + 1)
        $insideFence = $false
        $fence = ''
        $language = ''
        $lineNumber = 0
        $headings = [System.Collections.Generic.HashSet[string]]::new(
            [System.StringComparer]::OrdinalIgnoreCase
        )
        foreach ($line in Get-Content -LiteralPath $file.FullName) {
            $lineNumber += 1
            if ($line -match '[ \t]+$') {
                $issues.Add("$relativeFile`:$lineNumber has trailing whitespace")
            }
            if ($line.Contains("`t")) {
                $issues.Add("$relativeFile`:$lineNumber contains a tab")
            }

            if (-not $insideFence) {
                if ($line -match '^\s*(?<fence>```|~~~)(?<language>.*)$') {
                    $insideFence = $true
                    $fence = $Matches.fence
                    $language = $Matches.language.Trim().ToLowerInvariant()
                    continue
                }
                if ($line -match '^\s{0,3}#{1,6}\s+(?<heading>.+?)\s*#*\s*$') {
                    $heading = $Matches.heading
                    $heading = [regex]::Replace($heading, '\[([^\]]+)\]\([^)]+\)', '$1')
                    $heading = $heading.Replace('`', '')
                    $headingId = $heading.ToLowerInvariant()
                    $headingId = [regex]::Replace($headingId, '[^\p{L}\p{Nd}\s_-]', '')
                    $headingId = [regex]::Replace($headingId.Trim(), '\s+', '-')
                    if ($headingId -and -not $headings.Add($headingId)) {
                        $issues.Add(
                            "$relativeFile`:$lineNumber has duplicate heading id '$headingId'"
                        )
                    }
                }
                continue
            }

            if ($line -match ('^\s*' + [regex]::Escape($fence) + '\s*$')) {
                $insideFence = $false
                $fence = ''
                $language = ''
                continue
            }
            if ($language -eq '' -or $language -eq 'text') {
                if ($line.Length -gt 80) {
                    $issues.Add(
                        "$relativeFile`:$lineNumber text diagram exceeds 80 columns"
                    )
                }
                if ($line -match '[^\x00-\x7F]') {
                    $issues.Add(
                        "$relativeFile`:$lineNumber text diagram contains non-ASCII characters"
                    )
                }
            }
        }
        if ($insideFence) {
            $issues.Add("$relativeFile has an unclosed Markdown fence")
        }
    }
    if ($issues.Count -gt 0) {
        throw "package Markdown quality failures: $($issues -join '; ')"
    }
    Write-Host (
        'Package Markdown: structure, whitespace, heading IDs, and text-diagram ' +
        "quality passed for $($markdownFiles.Count) file(s)"
    )
}

function Test-MarkdownLinks([string]$packageRoot) {
    $broken = [System.Collections.Generic.List[string]]::new()
    $external = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::OrdinalIgnoreCase
    )
    $packagePrefix = $packageRoot.TrimEnd([System.IO.Path]::DirectorySeparatorChar) +
        [System.IO.Path]::DirectorySeparatorChar
    foreach ($file in Get-ChildItem -LiteralPath $packageRoot -Recurse -Filter '*.md' -File) {
        $text = Get-Content -LiteralPath $file.FullName -Raw
        foreach ($match in [regex]::Matches($text, '(?<!\!)\[[^\]]*\]\(([^)]+)\)')) {
            $target = $match.Groups[1].Value.Trim().Trim('<', '>')
            if ($target -match '^(https?://|mailto:)') {
                [void]$external.Add($target)
                continue
            }
            $parts = $target -split '#', 2
            $relativeTarget = $parts[0]
            $fragment = if ($parts.Count -eq 2) {
                [uri]::UnescapeDataString($parts[1])
            } else {
                $null
            }
            $resolved = if ($relativeTarget) {
                [System.IO.Path]::GetFullPath(
                    (Join-Path $file.DirectoryName ([uri]::UnescapeDataString($relativeTarget)))
                )
            } else {
                $file.FullName
            }
            if ($resolved -ne $packageRoot -and
                -not $resolved.StartsWith(
                    $packagePrefix,
                    [System.StringComparison]::OrdinalIgnoreCase
                )) {
                $relativeFile = $file.FullName.Substring($packageRoot.Length + 1)
                $broken.Add("$relativeFile -> $target (escapes package)")
                continue
            }
            if (-not (Test-Path -LiteralPath $resolved)) {
                $relativeFile = $file.FullName.Substring($packageRoot.Length + 1)
                $broken.Add("$relativeFile -> $target")
                continue
            }
            if ($fragment) {
                if (-not (Test-Path -LiteralPath $resolved -PathType Leaf) -or
                    [System.IO.Path]::GetExtension($resolved) -ne '.md') {
                    $relativeFile = $file.FullName.Substring($packageRoot.Length + 1)
                    $broken.Add("$relativeFile -> $target (fragment target is not Markdown)")
                    continue
                }
                $headingIds = Get-MarkdownHeadingIds $resolved
                if (-not $headingIds.Contains($fragment)) {
                    $relativeFile = $file.FullName.Substring($packageRoot.Length + 1)
                    $broken.Add("$relativeFile -> $target (heading not found)")
                }
            }
        }
    }
    if ($broken.Count -gt 0) {
        throw "broken package Markdown links: $($broken -join '; ')"
    }
    Write-Host (
        'Package Markdown: relative targets and heading fragments passed; ' +
        "$($external.Count) unique external link(s) skipped"
    )
}

function Read-JsonLine(
    [System.IO.StreamReader]$reader,
    [string]$description,
    [int]$timeoutMilliseconds = 10000
) {
    $task = $reader.ReadLineAsync()
    if (-not $task.Wait($timeoutMilliseconds)) {
        throw "$description timed out"
    }
    if ([string]::IsNullOrWhiteSpace($task.Result)) {
        throw "$description returned no data"
    }
    return $task.Result | ConvertFrom-Json
}

function New-NdjsonClient([int]$port) {
    $client = [System.Net.Sockets.TcpClient]::new('127.0.0.1', $port)
    $stream = $client.GetStream()
    $stream.ReadTimeout = 10000
    $stream.WriteTimeout = 10000
    $reader = [System.IO.StreamReader]::new(
        $stream,
        [System.Text.UTF8Encoding]::new($false)
    )
    $writer = [System.IO.StreamWriter]::new(
        $stream,
        [System.Text.UTF8Encoding]::new($false)
    )
    $writer.NewLine = [System.Environment]::NewLine
    $writer.AutoFlush = $true
    return [PSCustomObject]@{
        Client = $client
        Reader = $reader
        Writer = $writer
    }
}

function Send-WorkbenchRequest(
    $connection,
    [string]$callId,
    [string]$operation,
    [hashtable]$arguments
) {
    $request = @{
        v = 1
        type = 'request'
        call_id = $callId
        op = $operation
        args = $arguments
    } | ConvertTo-Json -Compress -Depth 16
    $connection.Writer.WriteLine($request)
    foreach ($frame in 1..64) {
        $response = Read-JsonLine $connection.Reader "Workbench $operation response"
        if ($response.PSObject.Properties.Name -contains 'call_id' -and
            $response.call_id -eq $callId) {
            return $response
        }
    }
    throw "Workbench $operation response exceeded the frame budget"
}

function Test-ExtractedPackage([string]$archivePath) {
    $temporaryBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
    $temporaryRoot = [System.IO.Path]::GetFullPath(
        (Join-Path $temporaryBase ("lab-runtime-preview-smoke-{0}" -f [guid]::NewGuid().ToString('N')))
    )
    Assert-ChildPath $temporaryRoot $temporaryBase
    [System.IO.Directory]::CreateDirectory($temporaryRoot) | Out-Null
    try {
        Expand-Archive -LiteralPath $archivePath -DestinationPath $temporaryRoot
        $packageRoot = Join-Path $temporaryRoot $packageName
        $runtimeExecutable = Join-Path $packageRoot 'lab-runtime.exe'
        $workbenchExecutable = Join-Path $packageRoot 'lab-workbench.exe'
        $starter = Join-Path $packageRoot 'examples\runtime.minimal.toml'
        $fullVirtual = Join-Path $packageRoot 'examples\runtime.virtual.toml'
        $babashkaExample = Join-Path $packageRoot 'clients\babashka-smoke\README.md'
        if (-not (Test-Path -LiteralPath $runtimeExecutable -PathType Leaf) -or
            -not (Test-Path -LiteralPath $workbenchExecutable -PathType Leaf) -or
            -not (Test-Path -LiteralPath $babashkaExample -PathType Leaf) -or
            -not (Test-Path -LiteralPath $starter -PathType Leaf) -or
            -not (Test-Path -LiteralPath $fullVirtual -PathType Leaf)) {
            throw 'extracted product binary, client example, or safe starter is missing'
        }

        foreach ($relativeExample in @(
            'examples\simple-device\read-only.json',
            'examples\simple-device\writable.json',
            'examples\simple-device\runtime.read-only.toml',
            'examples\simple-device\runtime.writable.toml'
        )) {
            $sourceExample = Join-Path $repositoryRoot $relativeExample
            $packageExample = Join-Path $packageRoot $relativeExample
            if (-not (Test-Path -LiteralPath $packageExample -PathType Leaf) -or
                (Get-FileHash -Algorithm SHA256 -LiteralPath $sourceExample).Hash -ne
                (Get-FileHash -Algorithm SHA256 -LiteralPath $packageExample).Hash) {
                throw "packaged SimpleDevice example differs from validated source: $relativeExample"
            }
        }

        $expectedFiles = @(Get-Content (Join-Path $packageRoot 'PACKAGE-CONTENTS.txt') |
            Sort-Object)
        $actualFiles = @(Get-ChildItem -LiteralPath $packageRoot -Recurse -File |
            ForEach-Object {
                $_.FullName.Substring($packageRoot.Length + 1).Replace('\', '/')
            } |
            Sort-Object)
        if (Compare-Object $expectedFiles $actualFiles) {
            throw 'extracted package does not match PACKAGE-CONTENTS.txt'
        }
        Test-MarkdownQuality $packageRoot
        Test-MarkdownLinks $packageRoot

        $executionRoot = Join-Path $temporaryRoot 'run'
        [System.IO.Directory]::CreateDirectory($executionRoot) | Out-Null
        $deploymentPath = Join-Path $executionRoot 'runtime.toml'
        Copy-Item -LiteralPath $starter -Destination $deploymentPath
        $runtimeProcess = $null
        $workbenchProcess = $null
        $runtimeConnection = $null
        $workbenchConnection = $null
        try {
            $runtimeStart = [System.Diagnostics.ProcessStartInfo]::new()
            $runtimeStart.FileName = $runtimeExecutable
            $runtimeStart.WorkingDirectory = $executionRoot
            $runtimeStart.UseShellExecute = $false
            $runtimeStart.CreateNoWindow = $true
            $runtimeStart.RedirectStandardOutput = $true
            $runtimeStart.RedirectStandardError = $true
            foreach ($argument in @(
                '--serve',
                '--config', $deploymentPath
            )) {
                [void]$runtimeStart.ArgumentList.Add($argument)
            }
            $runtimeStart.EnvironmentVariables['LAB_RUNTIME_LOG_DIRECTORY'] =
                (Join-Path $executionRoot 'logs')
            $runtimeProcess = [System.Diagnostics.Process]::new()
            $runtimeProcess.StartInfo = $runtimeStart
            if (-not $runtimeProcess.Start()) {
                throw 'extracted Runtime process did not start'
            }
            $runtimeErrorTask = $runtimeProcess.StandardError.ReadToEndAsync()
            $readiness = Read-JsonLine $runtimeProcess.StandardOutput 'Runtime readiness'
            if ($readiness.state -ne 'ready' -or [int]$readiness.port -lt 1) {
                throw "unexpected extracted Runtime readiness: $($readiness | ConvertTo-Json -Compress)"
            }
            if (-not (Test-Path -LiteralPath (Join-Path $executionRoot 'history.sqlite') -PathType Leaf)) {
                throw 'documented relative Recorder path did not resolve beside deployment'
            }

            $runtimeConnection = New-NdjsonClient ([int]$readiness.port)
            $runtimeConnection.Writer.WriteLine(
                '{"v":1,"msg_id":"preview-smoke-hello","op":"hello","args":{"scope":null}}'
            )
            $hello = Read-JsonLine $runtimeConnection.Reader 'Runtime hello'
            if ($hello.v -ne 1 -or $hello.msg_id -ne 'preview-smoke-hello' -or
                $hello.type -ne 'result' -or
                $hello.result.protocol.id -ne 'lab-runtime.application' -or
                $hello.result.protocol.version -ne 1 -or
                -not $hello.result.scope -or -not $hello.result.next_seq) {
                throw 'extracted Runtime hello failed'
            }
            $runtimeConnection.Writer.WriteLine(
                '{"v":1,"msg_id":"preview-smoke-reference","op":"reference","args":{"reference":"1"}}'
            )
            $reference = Read-JsonLine $runtimeConnection.Reader 'Runtime reference query'
            if ($reference.type -ne 'result' -or $reference.result.reference -ne '1' -or
                $reference.result.kind -ne 'fixed' -or -not $reference.result.revision -or
                $null -eq $reference.result.value) {
                throw 'extracted Runtime reference query failed'
            }

            $workbenchStart = [System.Diagnostics.ProcessStartInfo]::new()
            $workbenchStart.FileName = $workbenchExecutable
            $workbenchStart.WorkingDirectory = $executionRoot
            $workbenchStart.UseShellExecute = $false
            $workbenchStart.CreateNoWindow = $true
            $workbenchStart.RedirectStandardOutput = $true
            $workbenchStart.RedirectStandardError = $true
            foreach ($argument in @(
                '--connect', "127.0.0.1:$($readiness.port)",
                '--workspace', (Join-Path $executionRoot 'workspace'),
                '--workbench-listen', '127.0.0.1:0'
            )) {
                [void]$workbenchStart.ArgumentList.Add($argument)
            }
            $workbenchProcess = [System.Diagnostics.Process]::new()
            $workbenchProcess.StartInfo = $workbenchStart
            if (-not $workbenchProcess.Start()) {
                throw 'extracted Workbench process did not start'
            }
            $workbenchErrorTask = $workbenchProcess.StandardError.ReadToEndAsync()
            $workbenchReadiness = Read-JsonLine $workbenchProcess.StandardOutput 'Workbench endpoint readiness'
            $endpoint = [System.Net.IPEndPoint]::Parse(
                [string]$workbenchReadiness.workbench_endpoint
            )
            if ($endpoint.Address.ToString() -ne '127.0.0.1' -or $endpoint.Port -lt 1) {
                throw 'extracted Workbench published an unexpected endpoint'
            }

            $workbenchConnection = New-NdjsonClient $endpoint.Port
            $workbenchHello = Send-WorkbenchRequest $workbenchConnection 'package-hello' 'hello' @{}
            if ($workbenchHello.type -ne 'result' -or
                $workbenchHello.result.protocol.id -ne 'lab-runtime.workbench' -or
                $workbenchHello.result.protocol.version -ne 1) {
                throw 'extracted Workbench hello failed'
            }
            $workbenchReady = $false
            foreach ($attempt in 0..99) {
                $status = Send-WorkbenchRequest $workbenchConnection "package-status-$attempt" 'client_status' @{}
                if ($status.type -ne 'result') {
                    throw 'extracted Workbench status failed'
                }
                if ($status.result.runtime_client.connection -eq 'ready') {
                    $workbenchReady = $true
                    break
                }
                Start-Sleep -Milliseconds 20
            }
            if (-not $workbenchReady) {
                throw 'extracted Workbench did not connect to Runtime'
            }

            $closeRequested = $workbenchProcess.CloseMainWindow()
            if (-not $closeRequested -or -not $workbenchProcess.WaitForExit(5000)) {
                if (-not $workbenchProcess.HasExited) {
                    $workbenchProcess.Kill($true)
                }
                if (-not $workbenchProcess.WaitForExit(5000)) {
                    throw 'extracted Workbench cleanup timed out'
                }
                $workbenchStop = 'bounded process termination'
            } else {
                $workbenchStop = 'native window close'
            }
            [void]$workbenchErrorTask.GetAwaiter().GetResult()
            $workbenchProcess.Dispose()
            $workbenchProcess = $null
            if ($runtimeProcess.HasExited) {
                throw 'stopping extracted Workbench also stopped Runtime'
            }

            $shutdown = @{
                v = 1
                msg_id = 'preview-smoke-shutdown'
                op = 'runtime_shutdown'
                request_id = @{
                    scope = $hello.result.scope
                    seq = $hello.result.next_seq
                }
                args = @{}
            } | ConvertTo-Json -Compress -Depth 5
            $runtimeConnection.Writer.WriteLine($shutdown)
            $accepted = Read-JsonLine $runtimeConnection.Reader 'Runtime shutdown acceptance'
            $terminal = Read-JsonLine $runtimeConnection.Reader 'Runtime shutdown completion'
            if ($accepted.state -ne 'accepted' -or $terminal.state -ne 'completed') {
                throw "extracted Runtime shutdown failed: $($accepted.state)/$($terminal.state)"
            }
            if (-not $runtimeProcess.WaitForExit(10000)) {
                throw 'extracted Runtime process exit timed out'
            }
            $runtimeError = $runtimeErrorTask.GetAwaiter().GetResult()
            if ($runtimeProcess.ExitCode -ne 0) {
                throw "extracted Runtime exited $($runtimeProcess.ExitCode): $runtimeError"
            }
            $runtimeProcess.Dispose()
            $runtimeProcess = $null
            Write-Host (
                'Extracted smoke: Runtime hello/reference, Workbench hello/connect, ' +
                "Workbench stop via $workbenchStop, Runtime explicit shutdown passed"
            )
        }
        finally {
            if ($null -ne $workbenchConnection) {
                $workbenchConnection.Client.Dispose()
            }
            if ($null -ne $runtimeConnection) {
                $runtimeConnection.Client.Dispose()
            }
            if ($null -ne $workbenchProcess) {
                if (-not $workbenchProcess.HasExited) {
                    $workbenchProcess.Kill($true)
                    [void]$workbenchProcess.WaitForExit(5000)
                }
                $workbenchProcess.Dispose()
            }
            if ($null -ne $runtimeProcess) {
                if (-not $runtimeProcess.HasExited) {
                    $runtimeProcess.Kill($true)
                    [void]$runtimeProcess.WaitForExit(5000)
                }
                $runtimeProcess.Dispose()
            }
        }
    }
    finally {
        if (Test-Path -LiteralPath $temporaryRoot) {
            Remove-Item -LiteralPath $temporaryRoot -Recurse -Force
        }
    }
}

if ([System.Environment]::OSVersion.Platform -ne [System.PlatformID]::Win32NT) {
    throw 'developer-preview packaging currently supports Windows only'
}

Push-Location $repositoryRoot
try {
    $hostLine = (& rustc -vV | Where-Object { $_ -like 'host:*' })
    Assert-Success 'rustc host detection'
    $hostTriple = ($hostLine -replace '^host:\s*', '').Trim()
    if ($hostTriple -ne $targetTriple) {
        throw "expected Rust host $targetTriple, found $hostTriple"
    }

    $dirty = & git status --porcelain
    Assert-Success 'Git working-tree check'
    if (-not $AllowDirty -and $dirty) {
        throw 'working tree must be clean (use -AllowDirty only while developing the script)'
    }

    & cargo build --workspace --release --locked
    Assert-Success 'release build'

    $runtimeBinary = Join-Path $repositoryRoot 'target\release\lab-runtime.exe'
    $workbenchBinary = Join-Path $repositoryRoot 'target\release\lab-workbench.exe'
    foreach ($binary in @($runtimeBinary, $workbenchBinary)) {
        if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
            throw "required release binary is missing: $binary"
        }
    }

    Assert-ChildPath $stageRoot $distRoot
    Assert-ChildPath $zipPath $distRoot
    if (Test-Path -LiteralPath $stageRoot) {
        Remove-Item -LiteralPath $stageRoot -Recurse -Force
    }
    foreach ($artifact in @($zipPath, $checksumPath)) {
        if (Test-Path -LiteralPath $artifact) {
            Remove-Item -LiteralPath $artifact -Force
        }
    }
    [System.IO.Directory]::CreateDirectory($stageRoot) | Out-Null

    Copy-Item -LiteralPath $runtimeBinary -Destination (Join-Path $stageRoot 'lab-runtime.exe')
    Copy-Item -LiteralPath $workbenchBinary -Destination (Join-Path $stageRoot 'lab-workbench.exe')
    $publicFiles = @(
        'README.md',
        'LICENSE',
        'docs\getting-started.md',
        'docs\configuration.md',
        'docs\simple-device.md',
        'docs\developer\simple-device-tutorial.md',
        'docs\developer\full-driver-tutorial.md',
        'docs\architecture.md',
        'docs\application-api.md',
        'docs\recorder-sqlite.md',
        'docs\recovery-and-faults.md',
        'docs\safety-and-failures.md',
        'docs\extending-runtime.md',
        'docs\workbench.md',
        'docs\workbench-api.md',
        'docs\api\README.md',
        'docs\api\protocol-and-sessions.md',
        'docs\api\operations.md',
        'docs\api\events-mutations-and-recovery.md',
        'docs\api\errors-and-limits.md',
        'examples\runtime.minimal.toml',
        'examples\runtime.virtual.toml',
        'examples\simple-device\read-only.json',
        'examples\simple-device\writable.json',
        'examples\simple-device\runtime.read-only.toml',
        'examples\simple-device\runtime.writable.toml',
        'clients\babashka-smoke\README.md',
        'clients\babashka-smoke\run-smoke.ps1',
        'clients\babashka-smoke\runtime.clj',
        'clients\babashka-smoke\workbench.clj',
        'clients\clojurescript-smoke\README.md',
        'clients\clojurescript-smoke\run-smoke.ps1',
        'clients\clojurescript-smoke\src\lab_runtime_smoke\core.cljs'
    )
    foreach ($relativePath in $publicFiles) {
        Copy-ApprovedFile $relativePath $relativePath
    }

    $metadata = (& cargo metadata --format-version 1 --locked | ConvertFrom-Json)
    Assert-Success 'Cargo metadata'
    $runtimePackage = $metadata.packages |
        Where-Object { $_.name -eq 'lab-runtime' -and -not $_.source } |
        Select-Object -First 1
    $workbenchPackage = $metadata.packages |
        Where-Object { $_.name -eq 'lab-workbench' -and -not $_.source } |
        Select-Object -First 1
    if (-not $runtimePackage -or -not $workbenchPackage) {
        throw 'product workspace package metadata is missing'
    }
    $releaseBaseVersion = ($PreviewVersion.Substring(1) -split '-', 2)[0]
    if ($runtimePackage.version -ne $releaseBaseVersion -or
        $workbenchPackage.version -ne $releaseBaseVersion) {
        throw (
            "preview $PreviewVersion must use Cargo package version $releaseBaseVersion; " +
            "found Runtime $($runtimePackage.version), Workbench $($workbenchPackage.version)"
        )
    }
    $commit = (& git rev-parse HEAD).Trim()
    Assert-Success 'Git revision lookup'
    $sourceState = if ($dirty) { 'dirty' } else { 'clean' }
    $buildIdentity = @(
        'lab-runtime portable preview package'
        "preview-version=$PreviewVersion"
        "cargo-package-version=$($runtimePackage.version)"
        'products=lab-runtime.exe,lab-workbench.exe'
        "target=$targetTriple"
        "git-commit=$commit"
        "git-tree=$sourceState"
        'protocol=lab-runtime.application/1'
        'application-api=0.1-pre'
        'project-license=MIT'
        'runtime-dependency=Microsoft Visual C++ 2015-2022 x64 runtime (VCRUNTIME140.dll)'
    )
    [System.IO.File]::WriteAllLines(
        (Join-Path $stageRoot 'BUILD.txt'),
        $buildIdentity,
        $utf8NoBom
    )

    $treeLines = & cargo tree -p lab-runtime -p lab-workbench --target $targetTriple --edges normal --prefix none --format '{p}|{l}' --locked
    Assert-Success 'Windows dependency inventory'
    $dependencyEntries = @{}
    foreach ($line in $treeLines) {
        if ([string]::IsNullOrWhiteSpace($line)) {
            continue
        }
        if ($line -notmatch '^(?<name>[A-Za-z0-9_-]+) v(?<version>\S+?)(?: \([^)]*\))?\|.*$') {
            throw "cannot parse Cargo dependency inventory line: $line"
        }
        $name = $Matches.name
        $version = $Matches.version
        $workspacePackages = @($metadata.packages |
            Where-Object {
                $_.name -eq $name -and
                $_.version -eq $version -and
                -not $_.source
            })
        $externalPackages = @($metadata.packages |
            Where-Object {
                $_.name -eq $name -and
                $_.version -eq $version -and
                $_.source
            })
        if ($workspacePackages.Count -gt 0 -and $externalPackages.Count -eq 0) {
            continue
        }
        if ($externalPackages.Count -ne 1) {
            throw (
                "Cargo dependency inventory identity is missing or ambiguous: " +
                "$name $version ($($externalPackages.Count) external metadata matches)"
            )
        }
        $package = $externalPackages[0]
        $license = if ([string]::IsNullOrWhiteSpace([string]$package.license)) {
            '[missing Cargo license metadata]'
        } else {
            [string]$package.license
        }
        $dependencyEntries[$package.id] = [PSCustomObject]@{
            Name = $name
            Version = $version
            License = $license
            Package = $package
        }
    }
    $dependencies = @($dependencyEntries.Values | Sort-Object Name, Version)
    $missingLicenseMetadata = @($dependencies |
        Where-Object { $_.License -eq '[missing Cargo license metadata]' })
    $notice = [System.Collections.Generic.List[string]]::new()
    $notice.Add('THIRD-PARTY DEPENDENCY NOTICES')
    $notice.Add('')
    $notice.Add('Generated from Cargo.lock and the Windows x86_64 normal dependency graphs')
    $notice.Add('for lab-runtime.exe and lab-workbench.exe.')
    $notice.Add('The inventory covers every unique normal third-party dependency in those graphs.')
    $notice.Add('License expressions are copied from Cargo package metadata without interpretation.')
    $notice.Add('Copies of matching standalone top-level upstream license/notice files found by')
    $notice.Add('this packaging audit are included under licenses/.')
    $notice.Add('Absence from licenses/ does not assert that upstream provides no license text;')
    $notice.Add('unresolved entries are listed below for release/legal review.')
    $notice.Add('The lab-runtime workspace is licensed under MIT; see LICENSE at the package root.')
    $notice.Add('This file covers third-party dependencies and does not replace that project license.')
    $notice.Add('')
    $notice.Add("Third-party dependency count: $($dependencies.Count)")
    $notice.Add("Dependencies with missing Cargo license metadata: $($missingLicenseMetadata.Count)")
    if ($missingLicenseMetadata.Count -gt 0) {
        foreach ($dependency in $missingLicenseMetadata) {
            $notice.Add("- $($dependency.Name) $($dependency.Version)")
        }
    }
    $notice.Add('')
    $notice.Add('DEPENDENCY INVENTORY')
    $notice.Add('')
    foreach ($dependency in $dependencies) {
        $notice.Add("$($dependency.Name) $($dependency.Version) -- $($dependency.License)")
    }

    $missingLicenses = [System.Collections.Generic.List[string]]::new()
    foreach ($dependency in $dependencies) {
        $package = $dependency.Package
        $packageDirectory = Split-Path -Parent $package.manifest_path
        $licenseFiles = @(Get-ChildItem -LiteralPath $packageDirectory -File |
            Where-Object {
                $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE|UNLICENSE)(\.|-|$)'
            } |
            Sort-Object Name)
        if ($licenseFiles.Count -eq 0) {
            $missingLicenses.Add("$($dependency.Name) $($dependency.Version)")
            continue
        }
        $licenseDestination = Join-Path $stageRoot `
            ("licenses\{0}-{1}" -f $dependency.Name, $dependency.Version)
        [System.IO.Directory]::CreateDirectory($licenseDestination) | Out-Null
        foreach ($licenseFile in $licenseFiles) {
            Copy-Item -LiteralPath $licenseFile.FullName -Destination $licenseDestination
        }
    }
    if ($missingLicenses.Count -gt 0) {
        $notice.Add('')
        $notice.Add(
            'No matching standalone top-level license/notice file was found by the package audit for:'
        )
        foreach ($missing in $missingLicenses) {
            $notice.Add("- $missing")
        }
        $notice.Add(
            'These entries remain unresolved for release/legal review before external redistribution.'
        )
    }
    [System.IO.File]::WriteAllLines(
        (Join-Path $stageRoot 'THIRD-PARTY-NOTICES.txt'),
        $notice,
        $utf8NoBom
    )

    $contentManifestPath = Join-Path $stageRoot 'PACKAGE-CONTENTS.txt'
    $contentPaths = @(Get-ChildItem -LiteralPath $stageRoot -Recurse -File |
        ForEach-Object {
            $_.FullName.Substring($stageRoot.Length + 1).Replace('\', '/')
        })
    $contentPaths += 'PACKAGE-CONTENTS.txt'
    [System.IO.File]::WriteAllLines(
        $contentManifestPath,
        @($contentPaths | Sort-Object -Unique),
        $utf8NoBom
    )

    $commitTimeText = (& git show -s --format=%cI HEAD).Trim()
    Assert-Success 'Git commit-time lookup'
    $archiveTime = [System.DateTimeOffset]::Parse($commitTimeText).UtcDateTime
    $zipEpoch = [System.DateTime]::SpecifyKind(
        [System.DateTime]::new(1980, 1, 1),
        [System.DateTimeKind]::Utc
    )
    if ($archiveTime -lt $zipEpoch) {
        $archiveTime = $zipEpoch
    }
    foreach ($item in Get-ChildItem -LiteralPath $stageRoot -Recurse) {
        $item.LastWriteTimeUtc = $archiveTime
    }
    (Get-Item -LiteralPath $stageRoot).LastWriteTimeUtc = $archiveTime

    Add-Type -AssemblyName System.IO.Compression
    $zipStream = [System.IO.File]::Open(
        $zipPath,
        [System.IO.FileMode]::CreateNew,
        [System.IO.FileAccess]::ReadWrite,
        [System.IO.FileShare]::None
    )
    try {
        $archive = [System.IO.Compression.ZipArchive]::new(
            $zipStream,
            [System.IO.Compression.ZipArchiveMode]::Create,
            $true
        )
        try {
            foreach ($file in Get-ChildItem -LiteralPath $stageRoot -Recurse -File |
                Sort-Object { $_.FullName.Substring($stageRoot.Length + 1) }) {
                $relative = $file.FullName.Substring($stageRoot.Length + 1).Replace('\', '/')
                $entry = $archive.CreateEntry(
                    "$packageName/$relative",
                    [System.IO.Compression.CompressionLevel]::Optimal
                )
                $entry.LastWriteTime = [System.DateTimeOffset]::new($archiveTime)
                $sourceStream = $file.OpenRead()
                $entryStream = $entry.Open()
                try {
                    $sourceStream.CopyTo($entryStream)
                }
                finally {
                    $entryStream.Dispose()
                    $sourceStream.Dispose()
                }
            }
        }
        finally {
            $archive.Dispose()
        }
    }
    finally {
        $zipStream.Dispose()
    }
    if (-not (Test-Path -LiteralPath $zipPath -PathType Leaf)) {
        throw 'ZIP creation did not produce an artifact'
    }
    $hash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $checksumLine = "$hash  $([System.IO.Path]::GetFileName($zipPath))"
    [System.IO.File]::WriteAllText($checksumPath, "$checksumLine`n", $utf8NoBom)
    $verified = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($verified -ne $hash) {
        throw 'ZIP checksum verification failed'
    }
    Test-ExtractedPackage $zipPath

    Write-Host "Preview directory: $stageRoot"
    Write-Host "Preview ZIP:       $zipPath"
    Write-Host "SHA-256:           $hash"
    Write-Host 'Package contents:'
    foreach ($relativePath in Get-Content -LiteralPath $contentManifestPath) {
        Write-Host "  $relativePath"
    }
    Write-Host (
        'Extracted smoke:   Runtime hello/reference, Workbench connect/lifetime, ' +
        'finite cleanup, and Markdown target/fragment checks passed'
    )
}
finally {
    Pop-Location
}
