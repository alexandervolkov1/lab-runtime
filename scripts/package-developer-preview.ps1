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
. (Join-Path $PSScriptRoot 'release-license-evidence.ps1')

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

function Test-UserDocumentation([string]$packageRoot) {
    # Keep the first-use route self-contained and separate from integration docs.
    $userFiles = @(
        'README.md', 'docs\README.md', 'docs\getting-started.md',
        'docs\workbench.md', 'docs\recording.md', 'docs\configuration.md',
        'docs\linux-runtime.md', 'docs\distributed-workbench.md',
        'docs\troubleshooting.md'
    )
    foreach ($relative in $userFiles) {
        $path = Join-Path $packageRoot $relative
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "missing user manual page: $relative"
        }
        $text = Get-Content -LiteralPath $path -Raw
        if ($text -match '(?i)\b(Clojure(?:Script)?|Babashka|Tuna|M(?:12|13|14|15|16|17|18))\b' -or
            $text -match '(?im)^\s*cargo\s+(build|run|test|clippy|fmt)\b' -or
            $text -match '(?i)external review|mutation identity|client worker|SQL receipts') {
            throw "developer-only material in user manual: $relative"
        }
        foreach ($block in [regex]::Matches($text, '(?ms)^```powershell\r?\n(.*?)^```')) {
            $parseTokens = $null
            $parseErrors = $null
            [void][Management.Automation.Language.Parser]::ParseInput(
                $block.Groups[1].Value, [ref]$parseTokens, [ref]$parseErrors)
            if ($parseErrors.Count) { throw "invalid PowerShell example: $relative" }
        }
    }
    foreach ($relative in @(
        'LICENSE', 'examples\runtime.minimal.toml', 'examples\runtime.virtual.toml',
        'examples\simple-device\read-only.json',
        'examples\simple-device\runtime.read-only.toml'
    )) {
        if (-not (Test-Path -LiteralPath (Join-Path $packageRoot $relative) -PathType Leaf)) {
            throw "missing documented package input: $relative"
        }
    }
    foreach ($relative in @('clients', 'ai', 'docs\developer', 'docs\api', 'docs\reference')) {
        if (Test-Path -LiteralPath (Join-Path $packageRoot $relative)) {
            throw "developer-only directory in user package: $relative"
        }
    }
    Write-Host 'User manual: required pages, audience separation and PowerShell syntax passed'
}

function Get-UserPackageFiles([string]$sourceRoot) {
    $files = @(Get-Content -LiteralPath (Join-Path $sourceRoot 'scripts/user-package-files.json') -Raw |
        ConvertFrom-Json)
    if (-not $files.Count -or @($files | Sort-Object -Unique).Count -ne $files.Count) {
        throw 'Empty or duplicate user package inventory'
    }
    foreach ($relative in $files) {
        if ($relative -notmatch '^(README\.md|LICENSE|docs/[a-z-]+\.md|docs/README\.md|examples/[a-z./-]+\.(toml|json))$' -or
            '..' -in ($relative -split '/') -or
            -not (Test-Path -LiteralPath (Join-Path $sourceRoot $relative) -PathType Leaf)) {
            throw "Invalid user package input: $relative"
        }
    }
    return $files
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
        if (-not (Test-Path -LiteralPath $runtimeExecutable -PathType Leaf) -or
            -not (Test-Path -LiteralPath $workbenchExecutable -PathType Leaf) -or
            -not (Test-Path -LiteralPath $starter -PathType Leaf) -or
            -not (Test-Path -LiteralPath $fullVirtual -PathType Leaf)) {
            throw 'extracted product binary or safe starter is missing'
        }

        foreach ($relativeExample in @(
            'examples\simple-device\read-only.json',
            'examples\simple-device\runtime.read-only.toml'
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
        Test-ExtractedLicenseEvidence $packageRoot $releaseLicenseRows
        Test-ReleasePackageHygiene $packageRoot
        Test-UserDocumentation $packageRoot
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
    $releaseLicenseRows = Get-ReleaseLicenseEvidence $repositoryRoot

    foreach ($output in @($stageRoot, $zipPath, $checksumPath)) {
        Assert-ChildPath $output $distRoot
        if (Test-Path -LiteralPath $output) {
            throw "release output already exists; preserve it and choose a new version: $output"
        }
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
    [System.IO.Directory]::CreateDirectory($stageRoot) | Out-Null

    Copy-Item -LiteralPath $runtimeBinary -Destination (Join-Path $stageRoot 'lab-runtime.exe')
    Copy-Item -LiteralPath $workbenchBinary -Destination (Join-Path $stageRoot 'lab-workbench.exe')
    $publicFiles = @(Get-UserPackageFiles $repositoryRoot)
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
        "cargo-lock-sha256=$((Get-FileHash -LiteralPath (Join-Path $repositoryRoot 'Cargo.lock')).Hash.ToLowerInvariant())"
        "lab-runtime.exe-sha256=$((Get-FileHash -LiteralPath $runtimeBinary).Hash.ToLowerInvariant())"
        "lab-workbench.exe-sha256=$((Get-FileHash -LiteralPath $workbenchBinary).Hash.ToLowerInvariant())"
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

    $supplementalEvidenceRoot = Join-Path $repositoryRoot 'third-party-licenses'
    $supplementalEvidenceSources = @{
        'accesskit-a55d3e1a18bb9ef0e4bccc9083fb13c3e0ad8969' = [PSCustomObject]@{
            Source = 'https://github.com/AccessKit/accesskit/tree/a55d3e1a18bb9ef0e4bccc9083fb13c3e0ad8969'
            Hashes = [ordered]@{
                'AUTHORS' = '3bab8c36f6a85657504aaefb37c7ff34e29b8c917290f3d28a1f0920c992e502'
                'LICENSE-APACHE' = '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a'
                'LICENSE-MIT' = '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3'
                'LICENSE.chromium' = '845022e0c1db1abb41a6ba4cd3c4b674ec290f3359d9d3c78ae558d4c0ed9308'
            }
        }
        'accesskit-1bbcf100942bac96c2c3a4a91cb67b0b20201a24' = [PSCustomObject]@{
            Source = 'https://github.com/AccessKit/accesskit/tree/1bbcf100942bac96c2c3a4a91cb67b0b20201a24'
            Hashes = [ordered]@{
                'AUTHORS' = '3bab8c36f6a85657504aaefb37c7ff34e29b8c917290f3d28a1f0920c992e502'
                'LICENSE-APACHE' = '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a'
                'LICENSE-MIT' = '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3'
                'LICENSE.chromium' = '845022e0c1db1abb41a6ba4cd3c4b674ec290f3359d9d3c78ae558d4c0ed9308'
            }
        }
        'clipboard-win-3b27cf2bfd1adcfa6e0264eb51c1025ddaf0f342' = [PSCustomObject]@{
            Source = 'https://github.com/DoumanAsh/clipboard-win/tree/3b27cf2bfd1adcfa6e0264eb51c1025ddaf0f342'
            Hashes = [ordered]@{
                'LICENSE' = 'c9bff75738922193e67fa726fa225535870d2aa1059f91452c411736284ad566'
            }
        }
        'egui-49682f8baa058bf49e011035cfbd6e825f88a5ef' = [PSCustomObject]@{
            Source = 'https://github.com/emilk/egui/tree/49682f8baa058bf49e011035cfbd6e825f88a5ef'
            Hashes = [ordered]@{
                'LICENSE-APACHE' = '8173d5c29b4f956d532781d2b86e4e30f83e6b7878dce18c919451d6ba707c90'
                'LICENSE-MIT' = '95ca92f5f8ea5231f1580b3a2a799e8260af3114b900e1def5355a7f44bcf60c'
                'epaint_default_fonts-0.36.2\emoji-icon-font-mit-license.txt' = 'b9d2c1d909aa149996fd4c91dcb92b2362a04431640c1d200959da94caf8cde1'
                'epaint_default_fonts-0.36.2\Hack-Regular.txt' = '47c0cccbeec7e8614548cc485588b28149e7874188df5f41b36efebcee285c87'
                'epaint_default_fonts-0.36.2\OFL.txt' = '6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2'
                'epaint_default_fonts-0.36.2\UFL.txt' = '2f0015108d68627bd788d313f529c21ff4da2c2c42a5e1f3883acc83480f9002'
            }
        }
        'egui_plot-c31b61732c1acb58268bb26c786e7664797602bd' = [PSCustomObject]@{
            Source = 'https://github.com/emilk/egui_plot/tree/c31b61732c1acb58268bb26c786e7664797602bd'
            Hashes = [ordered]@{
                'LICENSE-APACHE' = '8173d5c29b4f956d532781d2b86e4e30f83e6b7878dce18c919451d6ba707c90'
                'LICENSE-MIT' = '23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3'
            }
        }
        'profiling-8271551172eb6fa4cba47369aedd93790c623df9' = [PSCustomObject]@{
            Source = 'https://github.com/aclysma/profiling/tree/8271551172eb6fa4cba47369aedd93790c623df9'
            Hashes = [ordered]@{
                'LICENSE-APACHE' = '10d30a673cd5e9349bdc02aeb48f14b3386d27d0da32df8f0a555d4aa16aa551'
                'LICENSE-MIT' = 'c8167fdeeed46d3f244d3f85c5bf998ce889343691c32be2c61a8bc4b5c08333'
            }
        }
    }
    foreach ($sourceKey in @($supplementalEvidenceSources.Keys | Sort-Object)) {
        $source = $supplementalEvidenceSources[$sourceKey]
        $sourceDirectory = Join-Path $supplementalEvidenceRoot $sourceKey
        if (-not (Test-Path -LiteralPath $sourceDirectory -PathType Container)) {
            throw "supplemental license evidence directory is missing: $sourceDirectory"
        }
        $actualFiles = @(Get-ChildItem -LiteralPath $sourceDirectory -Recurse -File |
            ForEach-Object {
                $_.FullName.Substring($sourceDirectory.Length + 1)
            })
        $expectedFiles = @($source.Hashes.Keys)
        $difference = @(Compare-Object $expectedFiles $actualFiles)
        if ($difference.Count -ne 0) {
            throw "supplemental license evidence inventory changed: $sourceKey"
        }
        foreach ($relativePath in $expectedFiles) {
            $evidencePath = Join-Path $sourceDirectory $relativePath
            $actualHash = (Get-FileHash -LiteralPath $evidencePath -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($actualHash -ne $source.Hashes[$relativePath]) {
                throw "supplemental license evidence hash mismatch: $sourceKey/$relativePath"
            }
        }
    }

    $dualLicenseFiles = [ordered]@{
        'LICENSE-APACHE' = 'LICENSE-APACHE'
        'LICENSE-MIT' = 'LICENSE-MIT'
    }
    $accessKitFiles = [ordered]@{
        'AUTHORS' = 'AUTHORS'
        'LICENSE-APACHE' = 'LICENSE-APACHE'
        'LICENSE-MIT' = 'LICENSE-MIT'
        'LICENSE.chromium' = 'LICENSE.chromium'
    }
    $packageLicenseEvidence = [ordered]@{
        'accesskit@0.24.1' = [PSCustomObject]@{
            SourceKey = 'accesskit-a55d3e1a18bb9ef0e4bccc9083fb13c3e0ad8969'
            Files = $accessKitFiles
        }
        'accesskit_consumer@0.35.0' = [PSCustomObject]@{
            SourceKey = 'accesskit-1bbcf100942bac96c2c3a4a91cb67b0b20201a24'
            Files = $accessKitFiles
        }
        'accesskit_windows@0.32.1' = [PSCustomObject]@{
            SourceKey = 'accesskit-1bbcf100942bac96c2c3a4a91cb67b0b20201a24'
            Files = $accessKitFiles
        }
        'accesskit_winit@0.32.2' = [PSCustomObject]@{
            SourceKey = 'accesskit-1bbcf100942bac96c2c3a4a91cb67b0b20201a24'
            Files = [ordered]@{
                'AUTHORS' = 'AUTHORS'
                'LICENSE-APACHE' = 'LICENSE-APACHE'
            }
        }
        'clipboard-win@5.4.1' = [PSCustomObject]@{
            SourceKey = 'clipboard-win-3b27cf2bfd1adcfa6e0264eb51c1025ddaf0f342'
            Files = [ordered]@{ 'LICENSE' = 'LICENSE' }
        }
        'ecolor@0.36.2' = [PSCustomObject]@{
            SourceKey = 'egui-49682f8baa058bf49e011035cfbd6e825f88a5ef'
            Files = $dualLicenseFiles
        }
        'eframe@0.36.2' = [PSCustomObject]@{
            SourceKey = 'egui-49682f8baa058bf49e011035cfbd6e825f88a5ef'
            Files = $dualLicenseFiles
        }
        'egui@0.36.2' = [PSCustomObject]@{
            SourceKey = 'egui-49682f8baa058bf49e011035cfbd6e825f88a5ef'
            Files = $dualLicenseFiles
        }
        'egui_glow@0.36.2' = [PSCustomObject]@{
            SourceKey = 'egui-49682f8baa058bf49e011035cfbd6e825f88a5ef'
            Files = $dualLicenseFiles
        }
        'egui_plot@0.37.0' = [PSCustomObject]@{
            SourceKey = 'egui_plot-c31b61732c1acb58268bb26c786e7664797602bd'
            Files = $dualLicenseFiles
        }
        'egui-winit@0.36.2' = [PSCustomObject]@{
            SourceKey = 'egui-49682f8baa058bf49e011035cfbd6e825f88a5ef'
            Files = $dualLicenseFiles
        }
        'emath@0.36.2' = [PSCustomObject]@{
            SourceKey = 'egui-49682f8baa058bf49e011035cfbd6e825f88a5ef'
            Files = $dualLicenseFiles
        }
        'epaint@0.36.2' = [PSCustomObject]@{
            SourceKey = 'egui-49682f8baa058bf49e011035cfbd6e825f88a5ef'
            Files = $dualLicenseFiles
        }
        'epaint_default_fonts@0.36.2' = [PSCustomObject]@{
            SourceKey = 'egui-49682f8baa058bf49e011035cfbd6e825f88a5ef'
            Files = [ordered]@{
                'LICENSE-APACHE' = 'LICENSE-APACHE'
                'LICENSE-MIT' = 'LICENSE-MIT'
                'epaint_default_fonts-0.36.2\emoji-icon-font-mit-license.txt' = 'fonts\emoji-icon-font-mit-license.txt'
                'epaint_default_fonts-0.36.2\Hack-Regular.txt' = 'fonts\Hack-Regular.txt'
                'epaint_default_fonts-0.36.2\OFL.txt' = 'fonts\OFL.txt'
                'epaint_default_fonts-0.36.2\UFL.txt' = 'fonts\UFL.txt'
            }
        }
        'profiling@1.0.18' = [PSCustomObject]@{
            SourceKey = 'profiling-8271551172eb6fa4cba47369aedd93790c623df9'
            Files = $dualLicenseFiles
        }
    }
    $notice = [System.Collections.Generic.List[string]]::new()
    $notice.Add('THIRD-PARTY DEPENDENCY NOTICES')
    $notice.Add('')
    $notice.Add('Generated from Cargo.lock and the Windows x86_64 normal dependency graphs')
    $notice.Add('for lab-runtime.exe and lab-workbench.exe.')
    $notice.Add('The inventory covers every unique normal third-party dependency in those graphs.')
    $notice.Add('Rust standard-library evidence is separate: see NOTICE.txt and LICENSE-EVIDENCE.json.')
    $notice.Add('License expressions are copied from Cargo package metadata without interpretation.')
    $notice.Add('Copies of matching upstream license/notice files found by this packaging audit')
    $notice.Add('or recorded by exact-version evidence review are included under licenses/.')
    $notice.Add('Absence from licenses/ does not assert that upstream provides no license text;')
    $notice.Add('any unresolved entries are listed below for release/legal review.')
    $notice.Add('This evidence inventory does not make a legal-compliance determination.')
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
    $supplementalEntries = [System.Collections.Generic.List[object]]::new()
    $usedSupplementalKeys = [System.Collections.Generic.HashSet[string]]::new()
    foreach ($dependency in $dependencies) {
        $package = $dependency.Package
        $packageDirectory = Split-Path -Parent $package.manifest_path
        $packageKey = "$($dependency.Name)@$($dependency.Version)"
        $supplemental = $packageLicenseEvidence[$packageKey]
        if ($null -ne $supplemental) {
            $source = $supplementalEvidenceSources[$supplemental.SourceKey]
            $sourceDirectory = Join-Path $supplementalEvidenceRoot $supplemental.SourceKey
            $licenseDestination = Join-Path $stageRoot `
                ("licenses\{0}-{1}" -f $dependency.Name, $dependency.Version)
            [System.IO.Directory]::CreateDirectory($licenseDestination) | Out-Null
            $packagedFiles = [System.Collections.Generic.List[string]]::new()
            foreach ($sourceRelative in @($supplemental.Files.Keys | Sort-Object)) {
                $destinationRelative = $supplemental.Files[$sourceRelative]
                $destinationPath = Join-Path $licenseDestination $destinationRelative
                Assert-ChildPath $destinationPath $licenseDestination
                [System.IO.Directory]::CreateDirectory(
                    (Split-Path -Parent $destinationPath)
                ) | Out-Null
                Copy-Item `
                    -LiteralPath (Join-Path $sourceDirectory $sourceRelative) `
                    -Destination $destinationPath
                $packagedFiles.Add(
                    ("licenses/{0}-{1}/{2}" -f `
                        $dependency.Name,
                        $dependency.Version,
                        $destinationRelative.Replace('\', '/'))
                )
            }
            [void]$usedSupplementalKeys.Add($packageKey)
            $supplementalEntries.Add([PSCustomObject]@{
                Name = $dependency.Name
                Version = $dependency.Version
                Source = $source.Source
                Files = @($packagedFiles)
            })
            continue
        }
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
    $unusedSupplementalKeys = @($packageLicenseEvidence.Keys |
        Where-Object { -not $usedSupplementalKeys.Contains($_) })
    if ($unusedSupplementalKeys.Count -ne 0) {
        throw (
            'supplemental license evidence does not match the packaged dependency graph: ' +
            ($unusedSupplementalKeys -join ', ')
        )
    }
    $notice.Add('')
    $notice.Add('EXACT-VERSION SUPPLEMENTAL LICENSE-TEXT EVIDENCE')
    $notice.Add('')
    $notice.Add('Applicable upstream license/notice text was located and included for:')
    foreach ($entry in @($supplementalEntries | Sort-Object Name, Version)) {
        $notice.Add("- $($entry.Name) $($entry.Version)")
        $notice.Add("  source: $($entry.Source)")
        foreach ($file in $entry.Files) {
            $notice.Add("  file: $file")
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
    } else {
        $notice.Add('')
        $notice.Add('No dependency remains without included package-audit license/notice text evidence.')
        $notice.Add('Legal interpretation and release sign-off remain separate review decisions.')
    }
    if ($missingLicenses.Count -gt 0 -or $missingLicenseMetadata.Count -gt 0) {
        throw 'Unresolved dependency license evidence; refusing to create release archive'
    }
    [System.IO.File]::WriteAllLines(
        (Join-Path $stageRoot 'THIRD-PARTY-NOTICES.txt'),
        $notice,
        $utf8NoBom
    )

    foreach ($row in $releaseLicenseRows) {
        $destination = Get-ReleaseEvidenceDestination $row.path
        $destinationPath = Join-Path $stageRoot $destination
        if ((Test-Path -LiteralPath $destinationPath) -and
            (Get-FileHash -LiteralPath $destinationPath).Hash.ToLowerInvariant() -ne $row.sha256) {
            throw "Supplemental evidence conflicts with dependency source: $destination"
        }
        Copy-ApprovedFile "third-party-licenses/release/$($row.path)" $destination
    }
    $evidenceManifest = Get-ReleaseEvidenceManifest $releaseLicenseRows
    [IO.File]::WriteAllText((Join-Path $stageRoot 'LICENSE-EVIDENCE.json'),
        ($evidenceManifest | ConvertTo-Json -Depth 8) + "`n", $utf8NoBom)
    Test-ExtractedLicenseEvidence $stageRoot $releaseLicenseRows
    Test-ReleasePackageHygiene $stageRoot
    Test-UserDocumentation $stageRoot

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
