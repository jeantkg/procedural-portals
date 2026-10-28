param([string]$Toolchain = '')
$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    $env:CARGO_BUILD_JOBS = '2'
    $selection = @()
    if ($Toolchain) { $selection = @("+$Toolchain") }
    & cargo @selection test --locked
    if ($LASTEXITCODE -ne 0) { throw 'Tests failed' }
    & cargo @selection build --locked --release --target wasm32-unknown-unknown
    if ($LASTEXITCODE -ne 0) { throw 'WASM build failed' }
    New-Item -ItemType Directory -Force dist | Out-Null
    & cargo @selection run --locked --example pack -- target/wasm32-unknown-unknown/release/procedural_portals_mod.wasm dist/procedural-portals.wasm
    if ($LASTEXITCODE -ne 0) { throw 'Component validation/packaging failed' }
    Copy-Item default.settings.json dist/procedural-portals.settings.json
    Copy-Item README.md dist/README.md
    Copy-Item LICENSE dist/LICENSE
    New-Item -ItemType Directory -Force dist/docs | Out-Null
    Copy-Item docs/cached-terrain.md dist/docs/cached-terrain.md
    $version = (Select-String -Path Cargo.toml -Pattern '^version = "([^"]+)"').Matches[0].Groups[1].Value
    $releaseFiles = @('dist/procedural-portals.wasm', 'dist/procedural-portals.settings.json', 'dist/README.md', 'dist/LICENSE', 'dist/docs')
    Compress-Archive -LiteralPath $releaseFiles -DestinationPath "dist/procedural-portals-$version.zip" -Force
    Write-Host "Built $PSScriptRoot\dist\procedural-portals.wasm"
} finally { Pop-Location }
