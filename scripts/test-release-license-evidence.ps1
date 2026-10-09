# Lightweight packaging regressions: no Cargo build, network, GUI or user data.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'release-license-evidence.ps1')
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$temporaryParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$fixture = Join-Path $temporaryParent ('lab-license-test-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory((Join-Path $fixture 'third-party-licenses')) | Out-Null
Copy-Item -LiteralPath (Join-Path $repository 'third-party-licenses/release') -Destination (Join-Path $fixture 'third-party-licenses') -Recurse
Copy-Item -LiteralPath (Join-Path $repository 'Cargo.lock') -Destination $fixture
$script:passed = 0
function Expect-Failure([string]$Name, [scriptblock]$Action) {
    $rejected = $false
    try { & $Action | Out-Null } catch { $rejected = $true }
    if (-not $rejected) { throw "Regression accepted invalid input: $Name" }
    $script:passed++
    Write-Host "PASS reject: $Name"
}
function Corrupt-Then-Test([string]$Relative, [scriptblock]$Check) {
    $path = Join-Path $fixture $Relative
    $original = [IO.File]::ReadAllBytes($path)
    try {
        [IO.File]::WriteAllBytes($path, [byte[]]@(1,2,3))
        Expect-Failure $Relative $Check
    } finally { [IO.File]::WriteAllBytes($path, $original) }
}
try {
    $rows = Get-ReleaseLicenseEvidence $repository
    $fixtureRows = Get-ReleaseLicenseEvidence $fixture -SkipToolchain
    if ($rows.Count -ne $fixtureRows.Count) { throw 'Fixture differs' }
    $script:passed++
    $check = { Get-ReleaseLicenseEvidence $fixture -SkipToolchain }
    foreach ($relative in @('serialport-4.10.1/serialport-4.10.1.crate',
        'serialport-4.10.1/MPL-2.0.txt','serialport-4.10.1/SOURCE-CONTENTS.json',
        'rust-1.95.0/COPYRIGHT-library.html','rust-1.95.0/NOTICE.txt',
        'rust-1.95.0/compiler-builtins/LICENSE.txt','NOTICE.txt') +
        @('LICENSE','LICENSE-BoringSSL','LICENSE-other-bits','src/polyfill/once_cell/LICENSE-APACHE',
          'src/polyfill/once_cell/LICENSE-MIT','third_party/fiat/LICENSE','third_party/fiat/AUTHORS' |
            ForEach-Object { "ring-0.17.14/$_" })) {
        Corrupt-Then-Test "third-party-licenses/release/$relative" $check
    }
    Corrupt-Then-Test 'Cargo.lock' $check
    $manifestPath = Join-Path $fixture 'third-party-licenses/release/evidence.json'
    $originalManifest = [IO.File]::ReadAllBytes($manifestPath)
    try {
        $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
        $manifest.files = @($manifest.files | Where-Object { $_.path -ne 'ring-0.17.14/src/polyfill/once_cell/LICENSE-MIT' })
        [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 8))
        Expect-Failure 'missing manifest row even when source exists' $check
    } finally { [IO.File]::WriteAllBytes($manifestPath, $originalManifest) }
    try {
        $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
        $manifest.rust_release = '1.96.0'
        [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 8))
        Expect-Failure 'unreviewed toolchain evidence' $check
    } finally { [IO.File]::WriteAllBytes($manifestPath, $originalManifest) }
    $package = Join-Path $fixture 'package'
    foreach ($row in $rows) {
        $path = Join-Path $package (Get-ReleaseEvidenceDestination $row.path)
        [IO.Directory]::CreateDirectory((Split-Path -Parent $path)) | Out-Null
        Copy-Item -LiteralPath (Join-Path $repository "third-party-licenses/release/$($row.path)") -Destination $path
    }
    [IO.File]::WriteAllText((Join-Path $package 'LICENSE-EVIDENCE.json'), ((Get-ReleaseEvidenceManifest $rows) | ConvertTo-Json -Depth 8))
    Test-ExtractedLicenseEvidence $package $rows
    Test-ReleasePackageHygiene $package
    $script:passed++
    foreach ($relative in @('NOTICE.txt','licenses/serialport-4.10.1/serialport-4.10.1.crate',
        'licenses/rust-1.95.0/COPYRIGHT-library.html','licenses/ring-0.17.14/third_party/fiat/LICENSE',
        'LICENSE-EVIDENCE.json')) {
        Corrupt-Then-Test "package/$relative" { Test-ExtractedLicenseEvidence $package $rows }
    }
    [IO.File]::WriteAllText((Join-Path $package 'user.sqlite'), 'fixture')
    Expect-Failure 'user database in package' { Test-ReleasePackageHygiene $package }
    # Delete only the explicit fixture file created immediately above.
    Remove-Item -LiteralPath (Join-Path $package 'user.sqlite')
    [IO.File]::WriteAllText((Join-Path $package 'secret.txt'), ('ghp_' + ('X' * 36)))
    Expect-Failure 'token in package' { Test-ReleasePackageHygiene $package }
    Write-Host "Windows license packaging regressions: $script:passed PASS"
} finally {
    $resolved = [IO.Path]::GetFullPath($fixture)
    if (-not $resolved.StartsWith($temporaryParent.TrimEnd('\') + '\') -or
        (Split-Path -Leaf $resolved) -notmatch '^lab-license-test-[a-f0-9]{32}$') { throw 'Unsafe fixture cleanup path' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
