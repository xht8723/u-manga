param([switch]$Apply, [switch]$BuildOnly)
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..')).TrimEnd('\')
$report = Join-Path $workspace ('docs/storage-cleanup-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Path $report | Out-Null
$allowed = if ($BuildOnly) { @('target','dist') } else { @('target','dist','node_modules','.cache/cargo','.cache/pnpm') }
$targets = @($allowed | ForEach-Object { [IO.Path]::GetFullPath((Join-Path $workspace $_)) })
function Assert-Contained([string]$path, [string]$root) {
    $full = [IO.Path]::GetFullPath($path)
    if ($full -ne $root -and !$full.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)) { throw "Path escapes allowed root: $full" }
}
function Assert-Parents([string]$path) {
    $cursor = [IO.Path]::GetDirectoryName($path)
    while ($cursor -and $cursor.StartsWith($workspace, [StringComparison]::OrdinalIgnoreCase)) {
        if ([IO.Directory]::Exists($cursor) -and (([IO.File]::GetAttributes($cursor) -band [IO.FileAttributes]::ReparsePoint) -ne 0)) { throw "Reparse ancestor: $cursor" }
        if ($cursor -eq $workspace) { break }
        $cursor = [IO.Path]::GetDirectoryName($cursor)
    }
}
function Inventory([string]$root) {
    Assert-Contained $root $workspace
    Assert-Parents $root
    $stack = [Collections.Generic.Stack[string]]::new(); $stack.Push($root)
    while ($stack.Count) {
        $path = $stack.Pop(); Assert-Contained $path $root
        $item = Get-Item -LiteralPath $path -Force
        $link = ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
        if ($link) {
            # pnpm's generated internal links are removed as links, never traversed.
            if ($root -ne (Join-Path $workspace 'node_modules') -or !$item.Target) { throw "Unexpected reparse entry: $path" }
            $destination = if ([IO.Path]::IsPathRooted($item.Target)) { $item.Target } else { Join-Path $item.Parent.FullName $item.Target }
            Assert-Contained $destination $root
        }
        [pscustomobject]@{ path=$path; root=$root; directory=[bool]$item.PSIsContainer; link=$link; target=$item.Target; bytes=$(if (!$item.PSIsContainer -and !$link) { $item.Length } else { 0 }); modified=$item.LastWriteTimeUtc.Ticks }
        if ($item.PSIsContainer -and !$link) {
            foreach ($child in [IO.Directory]::EnumerateFileSystemEntries($path)) { $stack.Push($child) }
        }
    }
}
$manifest = [Collections.Generic.List[object]]::new()
$skipped = [Collections.Generic.List[object]]::new()
foreach ($root in $targets) {
    if (!(Test-Path -LiteralPath $root)) { continue }
    try {
        $entries = @(Inventory $root)
        foreach ($entry in $entries) { $manifest.Add($entry) }
        Write-Output "Inventoried $root ($($entries.Count) entries)"
    } catch { $skipped.Add(@{path=$root;reason=$_.Exception.Message}) }
}
$manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $report 'deletion-manifest.json')
if (!$Apply) { Write-Output "Dry-run manifest: $report"; return }

# Protect source, locks, embedded assets, and the published executable. Libraries,
# models, originals and benchmark evidence are outside every deletion root.
$protected = @()
foreach ($relative in @('src','src-tauri/src','src-tauri/Cargo.toml','src-tauri/build.rs','src-tauri/tauri.conf.json','crates','vendor','resources','assets','release','Cargo.toml','Cargo.lock','package.json','pnpm-lock.yaml')) {
    $path = Join-Path $workspace $relative
    if (!(Test-Path -LiteralPath $path)) { continue }
    foreach ($entry in @(Inventory $path)) {
        if (!$entry.directory -and !$entry.link) {
            $protected += [pscustomobject]@{path=$entry.path;sha256=(Get-FileHash -LiteralPath $entry.path -Algorithm SHA256).Hash}
        }
    }
}
$protected | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $report 'protected-before.json')
$cacheLogs = Get-ChildItem -LiteralPath (Join-Path $workspace '.cache') -File -ErrorAction SilentlyContinue
if ($cacheLogs) { Compress-Archive -LiteralPath $cacheLogs.FullName -DestinationPath (Join-Path $report 'diagnostic-logs.zip') }
$drive = [IO.DriveInfo]::new([IO.Path]::GetPathRoot($workspace))
$freeBefore = $drive.AvailableFreeSpace
$removedBytes = [long]0; $removedFiles = 0
foreach ($entry in ($manifest | Sort-Object { $_.path.Length } -Descending)) {
    try {
        Assert-Contained $entry.path $entry.root
        Assert-Parents $entry.path
        $item = Get-Item -LiteralPath $entry.path -Force
        $link = ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
        if ($link -ne $entry.link -or [bool]$item.PSIsContainer -ne $entry.directory) { throw 'Path kind changed after inventory' }
        if ($entry.link -and $item.Target -ne $entry.target) { throw 'Link target changed after inventory' }
        if (!$entry.directory -and !$entry.link -and ($item.Length -ne $entry.bytes -or $item.LastWriteTimeUtc.Ticks -ne $entry.modified)) { throw 'File changed after inventory' }
        if ($entry.directory) { [IO.Directory]::Delete($entry.path, $false) }
        else { Remove-Item -LiteralPath $entry.path -Force -ErrorAction Stop; $removedFiles++; $removedBytes += $entry.bytes }
    } catch { $skipped.Add(@{path=$entry.path;reason=$_.Exception.Message}) }
}
$mismatches = @($protected | Where-Object { !(Test-Path -LiteralPath $_.path) -or (Get-FileHash -LiteralPath $_.path -Algorithm SHA256).Hash -ne $_.sha256 })
$result = [ordered]@{
    roots=$targets; removedFiles=$removedFiles; removedApparentBytes=$removedBytes
    freeBefore=$freeBefore; freeAfter=$drive.AvailableFreeSpace
    observedRecoveredBytes=($drive.AvailableFreeSpace-$freeBefore)
    protectedFiles=$protected.Count; protectedHashMismatches=$mismatches
    skipped=$skipped; report=$report
}
$result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $report 'result.json')
Write-Output ([pscustomobject]$result | Select-Object removedFiles,removedApparentBytes,observedRecoveredBytes,protectedFiles,report | ConvertTo-Json)
if ($mismatches.Count) { throw 'Protected-file verification failed. Inspect result.json.' }
if ($skipped.Count) { Write-Output "$($skipped.Count) entries were skipped; see result.json. No unrelated processes were stopped." }
