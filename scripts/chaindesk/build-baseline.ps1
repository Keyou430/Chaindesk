param([switch]$PinnedMsvc, [switch]$Portable)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
. (Join-Path $PSScriptRoot 'bootstrap.ps1')
$env:NODE_OPTIONS = '--max-old-space-size=4096'
$buildName = if ($Portable) { 'portable' } else { 'baseline' }

$vsWhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vsRoot = & $vsWhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vsRoot) { throw 'Microsoft C++ build tools are required' }
Import-Module (Join-Path $vsRoot 'Common7/Tools/Microsoft.VisualStudio.DevShell.dll')
Enter-VsDevShell -VsInstallPath $vsRoot -SkipAutomaticLocation -DevCmdArguments '-arch=amd64 -host_arch=amd64 -vcvars_ver=14.44'
$cargoVerbosity = @()
if ($PinnedMsvc) {
    $prepared = & node (Join-Path $PSScriptRoot 'prepare-msvc.mjs')
    if ($LASTEXITCODE -ne 0) { throw 'Pinned MSVC preparation failed' }
    $toolset = ($prepared | ConvertFrom-Json).toolsetRoot
    $env:PATH = "$(Join-Path $toolset 'bin/Hostx64/x64');$env:PATH"
    $env:INCLUDE = "$(Join-Path $toolset 'include');$env:INCLUDE"
    $env:LIB = "$(Join-Path $toolset 'lib/x64');$env:LIB"
    $env:CC = Join-Path $toolset 'bin/Hostx64/x64/cl.exe'
    $env:CXX = $env:CC
    $env:VCToolsInstallDir = $toolset + [System.IO.Path]::DirectorySeparatorChar
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = Join-Path $toolset 'bin/Hostx64/x64/link.exe'
    $cargoVerbosity = @('--verbose')
}

$outputRoot = Join-Path $projectRoot '.chaindesk'
New-Item -ItemType Directory -Path (Join-Path $outputRoot 'logs') -Force | Out-Null
$compiler = Get-Item -LiteralPath (Get-Command cl.exe).Source
$linker = Get-Item -LiteralPath (Get-Command link.exe).Source
$environmentJson = [ordered]@{
    upstreamCommit = $baseline.source.commit
    pinnedMsvc = [bool]$PinnedMsvc
    compiler = @{ path = $compiler.FullName; version = $compiler.VersionInfo.ProductVersion; sha256 = (Get-FileHash -LiteralPath $compiler.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
    linker = @{ path = $linker.FullName; version = $linker.VersionInfo.ProductVersion; sha256 = (Get-FileHash -LiteralPath $linker.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
    windowsSdkDirectory = $env:WindowsSdkDir
    windowsSdkVersion = $env:WindowsSDKVersion.TrimEnd([System.IO.Path]::DirectorySeparatorChar)
    vsShellDirectory = $vsRoot
} | ConvertTo-Json -Depth 4
$environmentName = if ($Portable) { 'logs/portable-build-environment.json' } else { 'logs/build-environment.json' }
[System.IO.File]::WriteAllText((Join-Path $outputRoot $environmentName), $environmentJson, [System.Text.UTF8Encoding]::new($false))
$buildConfiguration = Join-Path $outputRoot "tauri-$buildName.json"
$configurationJson = @{ bundle = @{ active = $false; externalBin = @(); resources = @() } } | ConvertTo-Json -Depth 4
[System.IO.File]::WriteAllText($buildConfiguration, $configurationJson, [System.Text.UTF8Encoding]::new($false))

# Packaging and application startup remain separate from the native build.
Push-Location $projectRoot
try {
    & pnpm.exe exec tauri build --debug --no-bundle --config $buildConfiguration -- --locked -j 4 @cargoVerbosity *> (Join-Path $outputRoot "logs/$buildName-build.log")
    if ($LASTEXITCODE -ne 0) {
        Get-Content -LiteralPath (Join-Path $outputRoot "logs/$buildName-build.log") -Encoding UTF8 -Tail 45
        throw "$buildName build failed; see .chaindesk/logs/$buildName-build.log"
    }
    Get-Content -LiteralPath (Join-Path $outputRoot "logs/$buildName-build.log") -Encoding UTF8 -Tail 12
} finally {
    Pop-Location
}
