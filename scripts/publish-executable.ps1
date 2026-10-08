$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..')).TrimEnd('\')
$source = Join-Path $workspace 'target/release/u-manga.exe'
$release = Join-Path $workspace 'release'
$destination = Join-Path $release 'U-Manga.exe'
$manifestPath = Join-Path $release 'manifest.json'
if (!(Test-Path -LiteralPath $source -PathType Leaf)) { throw 'Validated packaged executable is missing.' }
New-Item -ItemType Directory -Path $release -Force | Out-Null
$expected = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
$temporary = Join-Path $release ('.U-Manga-' + [Guid]::NewGuid().ToString('N') + '.tmp')
Copy-Item -LiteralPath $source -Destination $temporary
if ((Get-FileHash -LiteralPath $temporary -Algorithm SHA256).Hash -ne $expected) { throw 'Publication copy verification failed.' }
try { Move-Item -LiteralPath $temporary -Destination $destination -Force }
catch {
    $destination = Join-Path $release ('U-Manga-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.exe')
    Move-Item -LiteralPath $temporary -Destination $destination
    Write-Output "Published alongside the running executable: $destination"
}
$exe = Get-Item -LiteralPath $destination
if ((Get-FileHash -LiteralPath $exe.FullName -Algorithm SHA256).Hash -ne $expected) { throw 'Published executable verification failed.' }
$manifest = @{ name='U-Manga'; version='0.1.0'; build='chapter-selection-jobs-2026-10-07'; detectedReadingOrder='rows-right-to-left-v1'; platform='windows-x64'; file=$exe.Name; bytes=$exe.Length; sha256=$expected.ToLower(); bundledModels=@('rtdetr_int8'); optionalModelsBundled=$false; cleanupModels=@('inpaint_migan','inpaint_manga_aot','inpaint_manga_lama'); preferencesLocation='app-config-root'; bookFormat=20; preferencesFormat=15; windowStateFormat=1; promptVersion=7; simplePromptVersion=4; structuredTextBatchVersion=1; simpleTranslation=$true; simpleTranslationDefault=$true; contextVersion=2; glossaryPromptVersion=9; glossaryBeforeTranslation=$true; batchGlossaryEditor=$true; hostedGlossaryEditor=$true; batchTranslationChoices=$true; omittedPages=$true; compactJobs=$true; compactMobile=$true; automaticNotifications=$true; localHosting=$true; hostingFormat=1; hostingApiVersion=1 }
$manifest | ConvertTo-Json | Set-Content -LiteralPath $manifestPath
$publication = @{ executable=$exe.FullName; bytes=$exe.Length; sha256=$expected.ToLower() }
$evidence = Join-Path $workspace 'test-output/chapter-selection-jobs'
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
$publication | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidence 'publication.json')
$publication | ConvertTo-Json
