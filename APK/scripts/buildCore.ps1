param(
    [ValidateSet('arm64-v8a', 'x86_64')]
    [string]$Abi = 'arm64-v8a',
    [string]$CacheRoot = (Join-Path $env:USERPROFILE '.cache\chaindesk-android'),
    [switch]$BuildDeviceTests
)

$ErrorActionPreference = 'Stop'
$apkRoot = Split-Path $PSScriptRoot -Parent
$manifest = Get-Content -LiteralPath (Join-Path $apkRoot 'toolchain.json') -Encoding UTF8 -Raw | ConvertFrom-Json
$goRoot = Join-Path $CacheRoot "unpacked\go-$($manifest.artifacts.go.version)\go"
$goExe = Join-Path $goRoot 'bin\go.exe'
$clang = Join-Path $CacheRoot "sdk\ndk\$($manifest.android.ndk)\toolchains\llvm\prebuilt\windows-x86_64\bin\clang.exe"
if (-not (Test-Path -LiteralPath $goExe) -or -not (Test-Path -LiteralPath $clang)) { throw 'Prepare the pinned Go and Android NDK toolchain first' }

$target = if ($Abi -eq 'arm64-v8a') { 'aarch64-linux-android' } else { 'x86_64-linux-android' }
$architecture = if ($Abi -eq 'arm64-v8a') { 'arm64' } else { 'amd64' }
$output = Join-Path $apkRoot "android\coreBridge\build\generated\jniLibs\$Abi"
New-Item -ItemType Directory -Path $output -Force | Out-Null
$settings = @{
    GOTOOLCHAIN = 'local'
    GOPATH = (Join-Path $CacheRoot 'go-path')
    GOMODCACHE = (Join-Path $CacheRoot 'go-mod')
    GOCACHE = (Join-Path $CacheRoot 'go-build')
    GOOS = 'android'
    GOARCH = $architecture
    CGO_ENABLED = '1'
    CC = "`"$clang`" --target=$target$($manifest.android.minSdk)"
}
$previous = @{}
try {
    foreach ($name in $settings.Keys) {
        $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        [Environment]::SetEnvironmentVariable($name, $settings[$name], 'Process')
    }
    & $goExe -C (Join-Path $apkRoot 'core') mod download
    if ($LASTEXITCODE -ne 0) { throw 'Go dependency download failed' }
    $moduleFile = & (Join-Path $PSScriptRoot 'preparePatchedCore.ps1') -ModuleCache $settings.GOMODCACHE -OutputDirectory (Join-Path $apkRoot 'android/coreBridge/build/patchedCore')
    $linkFlags = "-s -w -X github.com/metacubex/mihomo/constant.Version=v$($manifest.mihomo.version) -extldflags=-Wl,-z,max-page-size=16384,-soname,libchaindesk_core.so"
    & $goExe -C (Join-Path $apkRoot 'core') build -modfile $moduleFile -p 2 -trimpath -mod=readonly -buildmode=c-shared -tags 'cmfa,with_gvisor' -ldflags $linkFlags -o (Join-Path $output 'libchaindesk_core.so') ./bridge
    if ($LASTEXITCODE -ne 0) { throw "Native core build failed with exit code $LASTEXITCODE" }
    Get-FileHash -LiteralPath (Join-Path $output 'libchaindesk_core.so') -Algorithm SHA256
    if ($BuildDeviceTests) {
        $testDirectory = Join-Path $apkRoot 'android/coreBridge/build/deviceTests'
        New-Item -ItemType Directory -Path $testDirectory -Force | Out-Null
        & $goExe -C (Join-Path $apkRoot 'core') test -c -modfile $moduleFile -mod=readonly -p 2 -tags 'cmfa,with_gvisor' -o (Join-Path $testDirectory "chaindesk-core-tests-$Abi") ./engine
        if ($LASTEXITCODE -ne 0) { throw 'Android device test compilation failed' }
    }
}
finally {
    foreach ($name in $previous.Keys) { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
}
