# Teramind installer (Windows). Idempotent.
#
# Environment overrides (all optional):
#   $env:TERAMIND_VERSION         -- version tag to install
#   $env:TERAMIND_RELEASE_BASE    -- base URL for releases (default: https://get.teramind.dev)
#   $env:TERAMIND_INSTALL_ROOT    -- where binaries go (default: $env:LOCALAPPDATA\teramind)
#   $env:TERAMIND_NO_MODIFY_PATH  -- skip user PATH prepend (default: unset)

#Requires -Version 5
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Write-Info($msg) { Write-Information "install.ps1: $msg" -InformationAction Continue }
function Die($msg) { Write-Error "install.ps1: $msg"; exit 1 }

function Get-Triple {
    $arch = if ([Environment]::Is64BitOperatingSystem) {
        if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'aarch64' } else { 'x86_64' }
    } else { Die 'only 64-bit Windows is supported' }
    "${arch}-pc-windows-msvc"
}

function Resolve-Version($base) {
    if ($env:TERAMIND_VERSION) { return $env:TERAMIND_VERSION }
    $idx = Invoke-RestMethod -Uri "$base/releases.json" -UseBasicParsing
    if (-not $idx.latest) { Die "could not parse $base/releases.json" }
    return $idx.latest
}

function Test-Sha256($file, $expected) {
    $actual = (Get-FileHash $file -Algorithm SHA256).Hash.ToLower()
    if ($actual -ne $expected.ToLower()) {
        Die "checksum mismatch for $file (expected $expected, got $actual)"
    }
}

$Base = if ($env:TERAMIND_RELEASE_BASE) { $env:TERAMIND_RELEASE_BASE } else { 'https://get.teramind.dev' }
$InstallRoot = if ($env:TERAMIND_INSTALL_ROOT) { $env:TERAMIND_INSTALL_ROOT } else { Join-Path $env:LOCALAPPDATA 'teramind' }
$BinDir = Join-Path $InstallRoot 'bin'

$Triple  = Get-Triple
$Version = Resolve-Version $Base
Write-Info "installing teramind $Version for $Triple"

$ArchiveName = "teramind-$Version-$Triple.zip"
$ArchiveUrl  = "$Base/$Version/$ArchiveName"
$SumsUrl     = "$Base/$Version/teramind-$Version-SHA256SUMS"

$Tmp = New-Item -ItemType Directory -Path (Join-Path $env:TEMP "teramind-install-$([System.Guid]::NewGuid().Guid)")
try {
    $ArchivePath = Join-Path $Tmp $ArchiveName
    Write-Info "downloading $ArchiveUrl"
    Invoke-WebRequest -Uri $ArchiveUrl -OutFile $ArchivePath -UseBasicParsing

    $SumsPath = Join-Path $Tmp 'SHA256SUMS'
    Write-Info "downloading SHA256SUMS"
    Invoke-WebRequest -Uri $SumsUrl -OutFile $SumsPath -UseBasicParsing

    # Pluck the hex digest for our archive (format: "<sha>  <name>").
    $Line = Get-Content $SumsPath | Where-Object { $_ -match [regex]::Escape($ArchiveName) } | Select-Object -First 1
    if (-not $Line) { Die "no SHA256 entry for $ArchiveName in SHA256SUMS" }
    $Expected = ($Line -split '\s+')[0]
    Test-Sha256 $ArchivePath $Expected

    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    Expand-Archive -Path $ArchivePath -DestinationPath $BinDir -Force
    # The archive has a `teramind-<version>/` prefix; flatten.
    $Inner = Join-Path $BinDir "teramind-$Version"
    if (Test-Path $Inner) {
        Get-ChildItem -Path $Inner | Move-Item -Destination $BinDir -Force
        Remove-Item $Inner -Recurse -Force
    }

    if (-not $env:TERAMIND_NO_MODIFY_PATH) {
        $UserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if ($UserPath -notlike "*${BinDir}*") {
            [Environment]::SetEnvironmentVariable('Path', "${BinDir};$UserPath", 'User')
            Write-Info "prepended $BinDir to user PATH (open a new terminal to pick it up)"
        } else {
            Write-Info "user PATH already contains $BinDir"
        }
    }
    Write-Info ""
    Write-Info "next:  add the plugin in Claude Code:  /plugin marketplace add vahem-vmssoft/teramind"
    Write-Info "       then:  /plugin install teramind@teramind   (the daemon self-spawns on first session)"
} finally {
    Remove-Item $Tmp -Recurse -Force -ErrorAction SilentlyContinue
}
