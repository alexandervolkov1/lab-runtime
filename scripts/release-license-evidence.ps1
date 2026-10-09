# Offline, fail-closed checks for the pinned application/std license supplement.
# Dot-source from the Windows packager and its lightweight regression tests.
function Get-ReleaseLicenseEvidence([string]$RepositoryRoot, [switch]$SkipToolchain) {
    $base = Join-Path $RepositoryRoot 'third-party-licenses/release'
    $manifest = Get-Content -LiteralPath (Join-Path $base 'evidence.json') -Raw | ConvertFrom-Json
    if ($manifest.schema -ne 1 -or $manifest.rust_release -ne '1.95.0' -or
        $manifest.rust_commit -ne '59807616e1fa2540724bfbac14d7976d7e4a3860') {
        throw 'Unreviewed license evidence version'
    }
    $lockedCrates = @{
        serialport = @('4.10.1', '6ba5f8f29aa20853c4e3e85a33ec580eb66be1f057142e77a333834a318bacf2')
        ring = @('0.17.14', 'a4689e6c2294d81e88dc6261c768b63bc4fcdb852be6d1352498b114f61383b7')
    }
    $lock = Get-Content -LiteralPath (Join-Path $RepositoryRoot 'Cargo.lock') -Raw
    foreach ($name in $lockedCrates.Keys) {
        $blocks = @([regex]::Split($lock, '(?m)^\[\[package\]\]\r?$') |
            Where-Object { $_ -match ('(?m)^name = "' + [regex]::Escape($name) + '"\r?$') })
        $version = [regex]::Escape($lockedCrates[$name][0])
        $checksum = $lockedCrates[$name][1]
        if ($blocks.Count -ne 1 -or $blocks[0] -notmatch "(?m)^version = `"$version`"\r?$" -or
            $blocks[0] -notmatch "(?m)^checksum = `"$checksum`"\r?$" ) {
            throw "Unreviewed locked dependency: $name"
        }
    }
    $rows = @($manifest.files | Where-Object { 'windows' -in $_.targets })
    $required = @('NOTICE.txt', 'serialport-4.10.1/serialport-4.10.1.crate',
        'serialport-4.10.1/MPL-2.0.txt', 'serialport-4.10.1/SOURCE-CONTENTS.json',
        'serialport-4.10.1/LICENSE.txt', 'rust-1.95.0/COPYRIGHT-library.html',
        'rust-1.95.0/NOTICE.txt', 'rust-1.95.0/toolchain.json',
        'rust-1.95.0/compiler-builtins/LICENSE.txt', 'rust-1.95.0/compiler-builtins/libm/LICENSE.txt',
        'rust-1.95.0/STD-CARGO-LOCK.toml', 'rust-1.95.0/std-package.toml')
    $required += @('MIT','Apache-2.0','Unicode-3.0','BSD-2-Clause') | ForEach-Object { "rust-1.95.0/licenses/$_.txt" }
    $required += @('LICENSE', 'LICENSE-BoringSSL', 'LICENSE-other-bits',
        'src/polyfill/once_cell/LICENSE-APACHE', 'src/polyfill/once_cell/LICENSE-MIT',
        'third_party/fiat/LICENSE', 'third_party/fiat/AUTHORS') | ForEach-Object { "ring-0.17.14/$_" }
    if (@($rows.path | Sort-Object -Unique).Count -ne $rows.Count) { throw 'Duplicate evidence paths' }
    foreach ($path in $required) {
        if ($path -notin $rows.path) { throw "Missing required license evidence: $path" }
    }
    foreach ($row in $rows) {
        [void](Get-ReleaseEvidenceDestination $row.path)
        if ((Get-FileHash -LiteralPath (Join-Path $base $row.path) -Algorithm SHA256).Hash.ToLowerInvariant() -ne $row.sha256) {
            throw "Evidence hash mismatch: $($row.path)"
        }
    }
    if ((Get-FileHash -LiteralPath (Join-Path $base 'serialport-4.10.1/serialport-4.10.1.crate')).Hash.ToLowerInvariant() -ne $lockedCrates.serialport[1]) {
        throw 'MPL source archive differs from Cargo.lock'
    }
    if (-not $SkipToolchain) {
        $version = (& rustc -vV) -join "`n"
        if ($LASTEXITCODE -ne 0 -or $version -notmatch '(?m)^release: 1\.95\.0$' -or
            $version -notmatch '(?m)^commit-hash: 59807616e1fa2540724bfbac14d7976d7e4a3860$' -or
            $version -notmatch '(?m)^host: x86_64-pc-windows-msvc$') { throw 'Unreviewed Rust toolchain' }
        $sysroot = (& rustc --print sysroot).Trim()
        if ($LASTEXITCODE -ne 0) { throw 'Rust sysroot lookup failed' }
        $pinned = Get-Content -LiteralPath (Join-Path $base 'rust-1.95.0/toolchain.json') -Raw | ConvertFrom-Json
        if ((Get-FileHash -LiteralPath (Join-Path $sysroot 'share/doc/rust/COPYRIGHT-library.html')).Hash.ToLowerInvariant() -ne $pinned.copyright_library_sha256) {
            throw 'Installed Rust library notices differ from reviewed evidence'
        }
        foreach ($library in $pinned.targets.windows.rlibs) {
            $path = Join-Path $sysroot "lib/rustlib/x86_64-pc-windows-msvc/lib/$($library.file)"
            if ((Get-FileHash -LiteralPath $path).Hash.ToLowerInvariant() -ne $library.sha256) {
                throw "Unreviewed sysroot library: $($library.file)"
            }
        }
    }
    return ,$rows
}

function Get-ReleaseEvidenceDestination([string]$Path) {
    if ([IO.Path]::IsPathRooted($Path) -or $Path.Contains('\') -or '..' -in ($Path -split '/')) {
        throw "Unsafe evidence path: $Path"
    }
    if ($Path -eq 'NOTICE.txt') { return $Path }
    return "licenses/$Path"
}

function Get-ReleaseEvidenceManifest($Rows) {
    return [ordered]@{
        schema = 1; target = 'windows'; rust_release = '1.95.0'
        rust_commit = '59807616e1fa2540724bfbac14d7976d7e4a3860'
        scope = 'Supplemental application and standard-library evidence; see NOTICE.txt'
        files = @($Rows | ForEach-Object {
            [ordered]@{ path=(Get-ReleaseEvidenceDestination $_.path); sha256=$_.sha256; source=$_.source; targets=$_.targets }
        })
    }
}

function Test-ExtractedLicenseEvidence([string]$PackageRoot, $Rows) {
    $actual = Get-Content -LiteralPath (Join-Path $PackageRoot 'LICENSE-EVIDENCE.json') -Raw | ConvertFrom-Json
    $expected = Get-ReleaseEvidenceManifest $Rows
    if ($actual.schema -ne $expected.schema -or $actual.target -ne 'windows' -or
        $actual.rust_release -ne $expected.rust_release -or $actual.rust_commit -ne $expected.rust_commit -or
        $actual.files.Count -ne $expected.files.Count) { throw 'Extracted evidence inventory differs' }
    for ($i = 0; $i -lt $expected.files.Count; $i++) {
        $entry = $expected.files[$i]
        if ($actual.files[$i].path -ne $entry.path -or $actual.files[$i].sha256 -ne $entry.sha256 -or
            $actual.files[$i].source -ne $entry.source -or
            ($actual.files[$i].targets -join ',') -ne ($entry.targets -join ',')) { throw 'Extracted evidence row differs' }
        if ((Get-FileHash -LiteralPath (Join-Path $PackageRoot $entry.path)).Hash.ToLowerInvariant() -ne $entry.sha256) {
            throw "Extracted license/source missing or changed: $($entry.path)"
        }
    }
}

function Test-ReleasePackageHygiene([string]$PackageRoot) {
    foreach ($file in Get-ChildItem -LiteralPath $PackageRoot -File -Recurse -Force) {
        $relative = $file.FullName.Substring($PackageRoot.Length + 1).Replace('\','/')
        if ($relative -match '(?i)(^|/)(\.git|\.env|target)(/|$)|\.(sqlite(-wal|-shm)?|db|pdb|pem|key|pfx|log|tmp)$') {
            throw "Forbidden release package file: $relative"
        }
        # Executables and the checksum-pinned, unchanged upstream .crate are not
        # editable configuration. Inspect the distributed human-readable files.
        if ($file.Extension -notin @('.exe','.crate')) {
            $text = [IO.File]::ReadAllText($file.FullName)
            if ($text -match '-----BEGIN (RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----|gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,}|AKIA[A-Z0-9]{16}') {
                throw "Potential secret in release package: $relative"
            }
        }
    }
}
