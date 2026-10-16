[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^v0\.1\.0-preview\.[1-9][0-9]*$')]
    [string]$PreviewVersion
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
python (Join-Path $PSScriptRoot 'package-windows-runtime.py') --preview-version $PreviewVersion
if ($LASTEXITCODE -ne 0) { throw "Runtime packaging failed: $LASTEXITCODE" }
