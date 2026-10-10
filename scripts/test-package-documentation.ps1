[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory
)

# A lightweight projection of the shared user-file inventory.
# Does not build Rust, run product binaries, change existing archives or delete files.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$destination = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $destination) {
    throw 'Choose a new output directory; existing audit materials are preserved'
}
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'package-developer-preview.ps1'),
    [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw 'Packaging script has syntax errors' }
foreach ($name in @(
    'Get-MarkdownHeadingIds', 'Test-MarkdownQuality',
    'Test-MarkdownLinks', 'Test-UserDocumentation', 'Get-UserPackageFiles'
)) {
    $function = $ast.Find({ param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -eq $name
    }, $true)
    if (-not $function) { throw "Missing packaging function: $name" }
    Invoke-Expression $function.Extent.Text
}
$files = @(Get-UserPackageFiles $repositoryRoot)
$destinationPrefix = $destination.TrimEnd('\') + '\'
$sourcePrefix = $repositoryRoot.TrimEnd('\') + '\'
foreach ($relative in $files) {
    $source = [IO.Path]::GetFullPath((Join-Path $repositoryRoot $relative))
    $target = [IO.Path]::GetFullPath((Join-Path $destination $relative))
    if (-not $source.StartsWith($sourcePrefix, [StringComparison]::OrdinalIgnoreCase) -or
        -not $target.StartsWith($destinationPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Inventory path escapes its root: $relative"
    }
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $target))
    Copy-Item -LiteralPath $source -Destination $target
    if ((Get-FileHash -LiteralPath $source).Hash -ne (Get-FileHash -LiteralPath $target).Hash) {
        throw "Package document differs: $relative"
    }
}
Test-UserDocumentation $destination
Test-MarkdownQuality $destination
Test-MarkdownLinks $destination
# Check that an orphan hardware fixture cannot silently return to the user ZIP.
$orphan = Join-Path $destination 'examples/runtime.hardware.toml'
if (Test-Path -LiteralPath $orphan) { throw 'Unexpected fixture already exists' }
try {
    [IO.File]::WriteAllText($orphan, '# synthetic forbidden package fixture')
    $rejected = $false
    try { Test-UserDocumentation $destination } catch { $rejected = $true }
    if (-not $rejected) { throw 'User package accepted an extra hardware example' }
} finally {
    Remove-Item -LiteralPath $orphan
}
Write-Output 'PASS: extra hardware example rejected; clean projection restored'
Write-Output "PASS: $($files.Count) public package inputs; projection retained at $destination"
