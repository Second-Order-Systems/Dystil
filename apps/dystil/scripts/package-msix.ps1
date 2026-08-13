[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    # Release builds use Cargo's 0.0.<build> convention. Store/MSIX package
    # versions use the established 1.0.<build>.0 convention; four-part MSIX
    # versions are also accepted for local/manual packaging.
    [ValidatePattern('^(\d+\.\d+\.\d+|\d+\.\d+\.\d+\.\d+)$')]
    [string]$Version,

    [string]$CertThumbprint,

    [ValidateSet('release', 'release-dev', 'release-local')]
    [string]$ExecutableProfile = 'release',

    # CI uses a shared target directory and cross-target output layout. Omit
    # this for the normal local `src-tauri\\target\\release` layout.
    [string]$TargetTriple,

    [switch]$SkipSign
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$appRoot = Split-Path -Parent $PSScriptRoot
$tauriRoot = Join-Path $appRoot 'src-tauri'
$msixVersion = if ($Version -match '^0\.0\.(\d+)$') {
    "1.0.$($Matches[1]).0"
} elseif ($Version -match '^\d+\.\d+\.\d+\.\d+$') {
    $Version
} else {
    throw "Expected a 0.0.<build> application version or a four-part MSIX version, got '$Version'."
}
$cargoTargetRoot = if ($env:CARGO_TARGET_DIR) {
    $env:CARGO_TARGET_DIR
} else {
    Join-Path $tauriRoot 'target'
}
$targetProfileRoot = if ($TargetTriple) {
    Join-Path $cargoTargetRoot $TargetTriple
} else {
    $cargoTargetRoot
}
$releaseRoot = Join-Path $targetProfileRoot 'release'
$executableRoot = Join-Path $targetProfileRoot $ExecutableProfile
$stageRoot = Join-Path $tauriRoot 'target\msix-stage'
$packageRoot = Join-Path $tauriRoot 'target\msix'
$appVfs = Join-Path $stageRoot 'VFS\Local AppData\Dystil'
$assetsRoot = Join-Path $stageRoot 'Assets'

$sdkBin = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin' -Directory |
    Sort-Object Name -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64' } |
    Where-Object { Test-Path (Join-Path $_ 'makeappx.exe') } |
    Select-Object -First 1
if (-not $sdkBin) { throw 'MakeAppx.exe was not found in the Windows SDK.' }

$makeAppx = Join-Path $sdkBin 'makeappx.exe'
$signTool = Join-Path $sdkBin 'signtool.exe'

$requiredFiles = @(
    (Join-Path $executableRoot 'dystil-app.exe'),
    (Join-Path $releaseRoot 'bun.exe'),
    (Join-Path $tauriRoot 'dystil-mcp-x86_64-pc-windows-msvc.exe'),
    (Join-Path $releaseRoot 'libopenblas.dll'),
    (Join-Path $releaseRoot 'onnxruntime.dll'),
    (Join-Path $releaseRoot 'msvcp140.dll'),
    (Join-Path $releaseRoot 'msvcp140_1.dll'),
    (Join-Path $releaseRoot 'msvcp140_2.dll'),
    (Join-Path $releaseRoot 'vcruntime140.dll'),
    (Join-Path $releaseRoot 'vcruntime140_1.dll')
)
$missing = $requiredFiles | Where-Object { -not (Test-Path $_) }
if ($missing) { throw "Missing release files:`n$($missing -join "`n")" }

if (Test-Path $stageRoot) { Remove-Item -LiteralPath $stageRoot -Recurse -Force }
New-Item -ItemType Directory -Force -Path $appVfs, $assetsRoot, $packageRoot | Out-Null

Copy-Item (Join-Path $executableRoot 'dystil-app.exe') (Join-Path $appVfs 'dystil-app.exe')
Copy-Item (Join-Path $releaseRoot 'bun.exe') (Join-Path $appVfs 'bun.exe')
Copy-Item (Join-Path $tauriRoot 'dystil-mcp-x86_64-pc-windows-msvc.exe') (Join-Path $appVfs 'dystil-mcp.exe')
Get-ChildItem $releaseRoot -File -Filter '*.dll' | Copy-Item -Destination $appVfs
Copy-Item (Join-Path $releaseRoot 'assets') (Join-Path $appVfs 'assets') -Recurse

$iconRoot = Join-Path $tauriRoot 'icons'
$iconMap = @{
    'StoreLogo.png' = 'StoreLogo.png'
    'DYSTILAPP-Square44x44Logo.png' = 'Square44x44Logo.png'
    'DYSTILAPP-Square71x71Logo.png' = 'Square71x71Logo.png'
    'DYSTILAPP-Square150x150Logo.png' = 'Square150x150Logo.png'
    'DYSTILAPP-Square310x310Logo.png' = 'Square310x310Logo.png'
    'dystil.png' = '128x128.png'
}
foreach ($entry in $iconMap.GetEnumerator()) {
    Copy-Item (Join-Path $iconRoot $entry.Value) (Join-Path $assetsRoot $entry.Key)
}

$manifest = Get-Content (Join-Path $tauriRoot 'msix\AppxManifest.xml.template') -Raw
$manifest = $manifest.Replace('__VERSION__', $msixVersion).Replace('__ARCHITECTURE__', 'x64')
[System.IO.File]::WriteAllText((Join-Path $stageRoot 'AppxManifest.xml'), $manifest, [System.Text.UTF8Encoding]::new($false))

$package = Join-Path $packageRoot "SecondOrderSystems.Dystil_$msixVersion`_x64.msix"
if (Test-Path $package) { Remove-Item -LiteralPath $package -Force }
& $makeAppx pack /d $stageRoot /p $package /o
if ($LASTEXITCODE -ne 0) { throw "MakeAppx failed with exit code $LASTEXITCODE" }

if (-not $SkipSign) {
    if (-not $CertThumbprint) { throw 'CertThumbprint is required unless -SkipSign is supplied.' }
    & $signTool sign /fd SHA256 /sha1 $CertThumbprint /s My /tr 'http://timestamp.digicert.com' /td SHA256 $package
    if ($LASTEXITCODE -ne 0) { throw "SignTool failed with exit code $LASTEXITCODE" }
    & $signTool verify /pa /v $package
    if ($LASTEXITCODE -ne 0) { throw "Signature verification failed with exit code $LASTEXITCODE" }
}

Write-Host "MSIX package created: $package"
