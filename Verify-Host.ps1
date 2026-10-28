param([Parameter(Mandatory=$true)][string]$Component, [string]$Toolchain = '', [string]$Assets = '')
$ErrorActionPreference = 'Stop'
$portalComponent = (Resolve-Path -LiteralPath $Component).Path
$portalRoot = Join-Path $PSScriptRoot '../cinnabar'
if (-not $Assets) {
    $portalTarget = Get-Content (Join-Path $portalRoot 'assets/bedrock-target.json') -Raw | ConvertFrom-Json
    $portalCarrierName = Split-Path $portalTarget.artifacts.world_assets -Leaf
    $portalCandidates = @(
        (Join-Path $env:ProgramFiles ('Cinnabar/resources/assets/' + $portalCarrierName)),
        (Join-Path $env:LOCALAPPDATA ('Cinnabar/assets/compiled/' + $portalCarrierName)),
        (Join-Path $portalRoot $portalTarget.artifacts.world_assets)
    )
    $Assets = $portalCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (-not $Assets) { throw "Missing $portalCarrierName native world carrier; supply -Assets PATH" }
}
$portalAssets = (Resolve-Path -LiteralPath $Assets).Path
$portalExampleName = 'procedural_portals_check_' + [guid]::NewGuid().ToString('N')
$portalExamples = Join-Path $portalRoot 'crates/mod-host/examples'
$portalExample = Join-Path $portalExamples ($portalExampleName + '.rs')
$portalHadExamples = Test-Path -LiteralPath $portalExamples
New-Item -ItemType Directory -Force $portalExamples | Out-Null
$portalSelection = @()
if ($Toolchain) { $portalSelection = @("+$Toolchain") }
try {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'tools/host-check.rs') -Destination $portalExample
    Push-Location $portalRoot
    try {
        $env:CARGO_BUILD_JOBS = '2'
        & cargo @portalSelection run --locked -p mod-host --example $portalExampleName -- $portalComponent $portalAssets
        if ($LASTEXITCODE -ne 0) { throw 'Native host integration check failed' }
    } finally { Pop-Location }
} finally {
    Remove-Item -LiteralPath $portalExample -ErrorAction SilentlyContinue
    if (-not $portalHadExamples -and (Get-ChildItem -LiteralPath $portalExamples).Count -eq 0) {
        Remove-Item -LiteralPath $portalExamples
    }
}
