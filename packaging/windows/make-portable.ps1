<#
  Make a portable MediaDesk folder from a release build.
  Optional: -FfmpegDir C:\path\to\ffmpeg\bin copies ffmpeg.exe and ffprobe.exe next to the program.
#>
param(
  [string]$Bin = (Join-Path $PSScriptRoot '..\..\target\release'),
  [string]$Out = (Join-Path $PSScriptRoot '..\..\dist\MediaDesk-portable'),
  [string]$FfmpegDir = ''
)
$ErrorActionPreference = 'Stop'
$Root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$Exe = Join-Path $Bin 'mediadesk.exe'
if (-not (Test-Path $Exe)) { throw "mediadesk.exe not found in $Bin. Run: cargo build --release" }
if (Test-Path $Out) { Remove-Item -Recurse -Force $Out }
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Out = Resolve-Path $Out
Copy-Item $Exe (Join-Path $Out 'MediaDesk.exe')
Copy-Item (Join-Path $PSScriptRoot 'portable.txt') $Out
foreach ($f in 'LICENSE-MIT', 'LICENSE-APACHE') {
  $p = Join-Path $Root $f
  if (Test-Path $p) { Copy-Item $p $Out }
}
if ($FfmpegDir -ne '') {
  foreach ($t in 'ffmpeg.exe', 'ffprobe.exe') {
    $p = Join-Path $FfmpegDir $t
    if (Test-Path $p) { Copy-Item $p $Out } else { Write-Warning "$t not found in $FfmpegDir" }
  }
}
$Zip = "$Out.zip"
if (Test-Path $Zip) { Remove-Item -Force $Zip }
Compress-Archive -Path (Join-Path $Out '*') -DestinationPath $Zip
Write-Host "Portable MediaDesk: $Out"
Write-Host "Zip: $Zip"
