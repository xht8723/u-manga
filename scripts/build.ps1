param([switch]$PrepareAssets, [switch]$Offline, [string]$PythonExecutable, [switch]$KeepBuildCache)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$env:CARGO_HOME = Join-Path $PWD '.cache/cargo'
# Cargo discovers the linker independently, but Tauri also needs the SDK resource
# compiler in ordinary PowerShell sessions (outside a Visual Studio developer shell).
if (!$env:RC -and !(Get-Command rc.exe -ErrorAction SilentlyContinue)) {
    $sdkBin = Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'Windows Kits/10/bin'
    $resourceCompiler = Get-ChildItem -LiteralPath $sdkBin -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
        Sort-Object { [version]$_.Name } -Descending |
        ForEach-Object { Join-Path $_.FullName 'x64/rc.exe' } |
        Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (!$resourceCompiler) { throw 'Windows SDK resource compiler is missing. Install the Windows SDK or set RC to its rc.exe.' }
    $env:RC = $resourceCompiler
}
$pythonRuntime = $PythonExecutable
if (!$pythonRuntime) {
    $pythonCommand = Get-Command python -ErrorAction SilentlyContinue
    if ($pythonCommand) { $pythonRuntime = $pythonCommand.Source }
}
if (!$pythonRuntime) { throw 'Python is required for build-time notices. Pass -PythonExecutable with the path to a Python 3 interpreter; no benchmark packages are needed.' }
& $pythonRuntime -c 'import sys; assert sys.version_info >= (3, 10), "Python 3.10 or newer is required"'
if ($LASTEXITCODE) { throw 'Python interpreter check failed' }
if ($Offline -and $PrepareAssets) { throw '-PrepareAssets downloads build assets and cannot be combined with -Offline.' }
$pnpmStore = Join-Path $PWD '.cache/pnpm/store'
if ($Offline) { pnpm install --frozen-lockfile --store-dir $pnpmStore --offline }
else { pnpm install --frozen-lockfile --store-dir $pnpmStore }
if ($LASTEXITCODE) { throw 'Frontend dependency restoration failed' }
if (!$Offline) {
    cargo fetch --locked
    if ($LASTEXITCODE) { throw 'Rust dependency restoration failed' }
}
New-Item -ItemType Directory -Path test-output -Force | Out-Null
if ($PrepareAssets) {
    & $pythonRuntime scripts/bootstrap.py --only runtime
    if ($LASTEXITCODE) { throw 'Runtime preparation failed' }
    & $pythonRuntime scripts/bootstrap.py --only bundled-models
    if ($LASTEXITCODE) { throw 'Bundled detector preparation failed' }
    & $pythonRuntime scripts/bootstrap.py --only fonts
    if ($LASTEXITCODE) { throw 'Font preparation failed' }
}
& $pythonRuntime scripts/notices.py
if ($LASTEXITCODE) { throw 'Dependency notices failed' }
node node_modules/svelte-check/bin/svelte-check --tsconfig tsconfig.json
if ($LASTEXITCODE) { throw 'Frontend check failed' }
cargo test -p umanga-core --lib --tests --locked --offline
if ($LASTEXITCODE) { throw 'Core tests failed' }
node node_modules/vitest/vitest.mjs run
if ($LASTEXITCODE) { throw 'Frontend tests failed' }
pnpm build
if ($LASTEXITCODE) { throw 'Frontend build failed' }
cargo test -p u-manga --locked --offline
if ($LASTEXITCODE) { throw 'Desktop tests failed' }
node node_modules/@tauri-apps/cli/tauri.js build --no-bundle -- --locked --offline
if ($LASTEXITCODE) { throw 'Portable build failed' }
& (Join-Path $PSScriptRoot 'publish-executable.ps1')
if (!$KeepBuildCache) { & (Join-Path $PSScriptRoot 'cleanup-build-storage.ps1') -Apply -BuildOnly }

