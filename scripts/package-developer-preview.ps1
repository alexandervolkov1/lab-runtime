[CmdletBinding()]
param(
    [switch]$AllowDirty
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$packageName = 'lab-runtime-developer-preview-windows-x86_64'
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

function Test-MarkdownLinks([string]$packageRoot) {
    $broken = [System.Collections.Generic.List[string]]::new()
    foreach ($file in Get-ChildItem -LiteralPath $packageRoot -Recurse -Filter '*.md' -File) {
        $text = Get-Content -LiteralPath $file.FullName -Raw
        foreach ($match in [regex]::Matches($text, '(?<!\!)\[[^\]]*\]\(([^)]+)\)')) {
            $target = $match.Groups[1].Value.Trim().Trim('<', '>')
            if ($target -match '^(https?://|mailto:|#)') {
                continue
            }
            $target = ($target -split '#')[0]
            if (-not $target) {
                continue
            }
            $resolved = [System.IO.Path]::GetFullPath(
                (Join-Path $file.DirectoryName ([uri]::UnescapeDataString($target)))
            )
            if (-not (Test-Path -LiteralPath $resolved)) {
                $relativeFile = $file.FullName.Substring($packageRoot.Length + 1)
                $broken.Add("$relativeFile -> $target")
            }
        }
    }
    if ($broken.Count -gt 0) {
        throw "broken package Markdown links: $($broken -join '; ')"
    }
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
        $executable = Join-Path $packageRoot 'lab-runtime.exe'
        $starter = Join-Path $packageRoot 'examples\runtime.virtual.toml'
        if (-not (Test-Path -LiteralPath $executable -PathType Leaf) -or
            -not (Test-Path -LiteralPath $starter -PathType Leaf)) {
            throw 'extracted executable or safe starter configuration is missing'
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
        Test-MarkdownLinks $packageRoot

        $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $executable
        $startInfo.Arguments = '--serve --config .\examples\runtime.virtual.toml'
        $startInfo.WorkingDirectory = $packageRoot
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        $startInfo.EnvironmentVariables['LAB_RUNTIME_LOG_DIRECTORY'] =
            (Join-Path $temporaryRoot 'logs')
        $process = [System.Diagnostics.Process]::new()
        $process.StartInfo = $startInfo
        if (-not $process.Start()) {
            throw 'extracted preview process did not start'
        }
        try {
            $readinessTask = $process.StandardOutput.ReadLineAsync()
            if (-not $readinessTask.Wait(10000)) {
                throw 'extracted preview readiness timed out'
            }
            $readiness = $readinessTask.Result | ConvertFrom-Json
            if ($readiness.state -ne 'ready') {
                throw "unexpected extracted preview readiness: $($readinessTask.Result)"
            }

            $client = [System.Net.Sockets.TcpClient]::new(
                '127.0.0.1', [int]$readiness.port
            )
            try {
                $stream = $client.GetStream()
                $stream.ReadTimeout = 10000
                $stream.WriteTimeout = 10000
                $reader = [System.IO.StreamReader]::new(
                    $stream, [System.Text.UTF8Encoding]::new($false)
                )
                $writer = [System.IO.StreamWriter]::new(
                    $stream, [System.Text.UTF8Encoding]::new($false)
                )
                $writer.NewLine = "`n"
                $writer.AutoFlush = $true
                $writer.WriteLine(
                    '{"v":1,"msg_id":"preview-smoke-hello","op":"hello","args":{"scope":null}}'
                )
                $hello = $reader.ReadLine() | ConvertFrom-Json
                if ($hello.type -ne 'result') {
                    throw 'extracted preview hello failed'
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
                $writer.WriteLine($shutdown)
                $accepted = $reader.ReadLine() | ConvertFrom-Json
                $terminal = $reader.ReadLine() | ConvertFrom-Json
                if ($accepted.state -ne 'accepted' -or $terminal.state -ne 'completed') {
                    throw "extracted preview shutdown failed: $($accepted.state)/$($terminal.state)"
                }
            }
            finally {
                $client.Dispose()
            }
            if (-not $process.WaitForExit(10000)) {
                throw 'extracted preview process exit timed out'
            }
            if ($process.ExitCode -ne 0) {
                throw "extracted preview exited $($process.ExitCode): $($process.StandardError.ReadToEnd())"
            }
        }
        finally {
            if (-not $process.HasExited) {
                $process.Kill()
                $process.WaitForExit()
            }
            $process.Dispose()
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

    $binaryPath = Join-Path $repositoryRoot 'target\release\lab-runtime.exe'
    if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
        throw "release binary is missing: $binaryPath"
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

    Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $stageRoot 'lab-runtime.exe')
    Copy-ApprovedFile 'README.md' 'README.md'
    Copy-ApprovedFile 'docs\getting-started.md' 'docs\getting-started.md'
    Copy-ApprovedFile 'docs\architecture.md' 'docs\architecture.md'
    Copy-ApprovedFile 'docs\application-api.md' 'docs\application-api.md'
    Copy-ApprovedFile 'docs\recorder-sqlite.md' 'docs\recorder-sqlite.md'
    Copy-ApprovedFile 'docs\safety-and-failures.md' 'docs\safety-and-failures.md'
    Copy-ApprovedFile 'docs\extending-runtime.md' 'docs\extending-runtime.md'
    Copy-ApprovedFile 'examples\runtime.virtual.toml' 'examples\runtime.virtual.toml'

    $metadata = (& cargo metadata --format-version 1 --locked | ConvertFrom-Json)
    Assert-Success 'Cargo metadata'
    $runtimePackage = $metadata.packages |
        Where-Object { $_.name -eq 'lab-runtime' -and -not $_.source } |
        Select-Object -First 1
    if (-not $runtimePackage) {
        throw 'lab-runtime workspace package metadata is missing'
    }
    $commit = (& git rev-parse HEAD).Trim()
    Assert-Success 'Git revision lookup'
    $buildIdentity = @(
        'lab-runtime developer preview'
        "package-version=$($runtimePackage.version)"
        "target=$targetTriple"
        "git-commit=$commit"
        'protocol=lab-runtime.application/1'
        'application-api=0.1-pre'
        'runtime-dependency=Microsoft Visual C++ 2015-2022 x64 runtime (VCRUNTIME140.dll)'
    )
    [System.IO.File]::WriteAllLines(
        (Join-Path $stageRoot 'BUILD.txt'),
        $buildIdentity,
        $utf8NoBom
    )

    $treeLines = & cargo tree -p lab-runtime --target $targetTriple --edges normal `
        --prefix none --format '{p}|{l}' --locked
    Assert-Success 'Windows dependency inventory'
    $dependencyEntries = @{}
    foreach ($line in $treeLines) {
        if ($line -notmatch '^(?<name>[A-Za-z0-9_-]+) v(?<version>\S+?)(?: \([^)]*\))?\|(?<license>.+)$') {
            continue
        }
        $name = $Matches.name
        $version = $Matches.version
        $license = $Matches.license.Trim().Replace(' (*)', '')
        if (-not $license) {
            continue
        }
        $dependencyEntries["$name|$version"] = [PSCustomObject]@{
            Name = $name
            Version = $version
            License = $license
        }
    }
    $dependencies = @($dependencyEntries.Values | Sort-Object Name, Version)
    $notice = [System.Collections.Generic.List[string]]::new()
    $notice.Add('THIRD-PARTY DEPENDENCY NOTICES')
    $notice.Add('')
    $notice.Add('Generated from Cargo.lock and the Windows x86_64 normal dependency graph.')
    $notice.Add('SPDX expressions below come from upstream Cargo package metadata.')
    $notice.Add('Corresponding upstream license files are included under licenses/.')
    $notice.Add('The lab-runtime workspace itself does not currently declare a project license;')
    $notice.Add('this local developer-preview artifact does not imply a license grant.')
    $notice.Add('')
    foreach ($dependency in $dependencies) {
        $notice.Add("$($dependency.Name) $($dependency.Version) -- $($dependency.License)")
    }

    $missingLicenses = [System.Collections.Generic.List[string]]::new()
    foreach ($dependency in $dependencies) {
        $package = $metadata.packages |
            Where-Object {
                $_.name -eq $dependency.Name -and
                $_.version -eq $dependency.Version -and
                $_.source
            } |
            Select-Object -First 1
        if (-not $package) {
            $missingLicenses.Add("$($dependency.Name) $($dependency.Version)")
            continue
        }
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
        $notice.Add('No standalone upstream license file was present in the downloaded crate for:')
        foreach ($missing in $missingLicenses) {
            $notice.Add("- $missing")
        }
        $notice.Add('Consult that crate package metadata/source before external redistribution.')
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
    Write-Host 'Extracted smoke:   ready, hello, clean shutdown, link check passed'
}
finally {
    Pop-Location
}
