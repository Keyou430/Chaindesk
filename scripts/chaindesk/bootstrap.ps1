param([switch]$Install)

if ($PSVersionTable.PSVersion.Major -lt 7) {
    throw 'PowerShell 7 or later is required for the development scripts'
}

$previousErrorPreference = $ErrorActionPreference
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)

try {
    $baseline = Get-Content -LiteralPath (Join-Path $PSScriptRoot '../../docs/engineeringBaseline.json') -Encoding UTF8 -Raw | ConvertFrom-Json
    $cacheRoot = Join-Path ([Environment]::GetFolderPath('UserProfile')) '.cache/chaindesk/toolchains'
    $nodeRoot = Join-Path $cacheRoot "node-v$($baseline.toolchain.node)-win-x64"
    $nodeExecutable = Join-Path $nodeRoot 'node.exe'
    $pnpmRoot = Join-Path $cacheRoot 'pnpm/node_modules/@pnpm/exe.win32-x64'
    $pnpmExecutable = Join-Path $pnpmRoot 'pnpm.exe'
    $env:RUSTUP_HOME = Join-Path $cacheRoot 'rustup'
    $env:CARGO_HOME = Join-Path $cacheRoot 'cargo'

    if ($Install) {
        New-Item -ItemType Directory -Path $cacheRoot -Force | Out-Null
        $nodeArchive = Join-Path $cacheRoot $baseline.toolchainDownloads.node.archiveName
        if (-not (Test-Path -LiteralPath $nodeArchive)) {
            Invoke-WebRequest -Uri $baseline.toolchainDownloads.node.url -OutFile $nodeArchive
        }
        if ((Get-FileHash -LiteralPath $nodeArchive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $baseline.toolchainDownloads.node.sha256) {
            throw 'Node.js archive checksum mismatch'
        }
        if (-not (Test-Path -LiteralPath $nodeExecutable)) {
            Expand-Archive -LiteralPath $nodeArchive -DestinationPath $cacheRoot
        }
        if (-not (Test-Path -LiteralPath $pnpmExecutable)) {
            & $nodeExecutable (Join-Path $nodeRoot 'node_modules/npm/bin/npm-cli.js') install --prefix (Join-Path $cacheRoot 'pnpm') --ignore-scripts --no-audit --no-fund --no-package-lock --registry=https://registry.npmjs.org $baseline.toolchainDownloads.pnpm.package
            if ($LASTEXITCODE -ne 0) { throw 'Pinned pnpm installation failed' }
        }
        $rustupExecutable = Join-Path $env:CARGO_HOME 'bin/rustup.exe'
        $rustToolchain = Join-Path $env:RUSTUP_HOME "toolchains/$($baseline.toolchain.rust)-x86_64-pc-windows-msvc"
        if (-not (Test-Path -LiteralPath $rustupExecutable) -or -not (Test-Path -LiteralPath $rustToolchain)) {
            $rustupInstaller = Join-Path $cacheRoot 'rustup-init.exe'
            if (-not (Test-Path -LiteralPath $rustupInstaller)) {
                Invoke-WebRequest -Uri $baseline.toolchainDownloads.rustup.url -OutFile $rustupInstaller
            }
            if ((Get-FileHash -LiteralPath $rustupInstaller -Algorithm SHA256).Hash.ToLowerInvariant() -ne $baseline.toolchainDownloads.rustup.sha256) {
                throw 'Rustup checksum mismatch'
            }
            & $rustupInstaller -y --no-modify-path --profile minimal --default-toolchain $baseline.toolchain.rust --default-host x86_64-pc-windows-msvc --component rustfmt,clippy
            if ($LASTEXITCODE -ne 0) { throw 'Pinned Rust installation failed' }
        }
    }

    if (-not (Test-Path -LiteralPath $nodeExecutable) -or -not (Test-Path -LiteralPath $pnpmExecutable)) {
        throw 'Run this script with -Install to prepare the isolated toolchains'
    }
    if ((Get-FileHash -LiteralPath $pnpmExecutable -Algorithm SHA256).Hash.ToLowerInvariant() -ne $baseline.toolchainDownloads.pnpm.sha256) {
        throw 'pnpm executable checksum mismatch'
    }
    $env:PATH = "$nodeRoot;$pnpmRoot;$(Join-Path $env:CARGO_HOME 'bin');$env:PATH"
    $env:CARGO_TARGET_DIR = Join-Path $cacheRoot '../target'
    $env:HUSKY = '0'
    & $nodeExecutable (Join-Path $PSScriptRoot 'verify-baseline.mjs')
    if ($LASTEXITCODE -ne 0) { throw 'Toolchain or source baseline verification failed' }
} finally {
    $ErrorActionPreference = $previousErrorPreference
}
