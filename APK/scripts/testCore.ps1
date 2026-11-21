$ErrorActionPreference = 'Stop'
$apkRoot = Split-Path $PSScriptRoot -Parent
$cacheRoot = Join-Path $env:USERPROFILE '.cache/chaindesk-android'
$manifest = Get-Content -LiteralPath (Join-Path $apkRoot 'toolchain.json') -Encoding UTF8 -Raw | ConvertFrom-Json
$goExe = Join-Path $cacheRoot "unpacked/go-$($manifest.artifacts.go.version)/go/bin/go.exe"
$settings = @{
    GOTOOLCHAIN = 'local'
    GOPATH = (Join-Path $cacheRoot 'go-path')
    GOMODCACHE = (Join-Path $cacheRoot 'go-mod')
    GOCACHE = (Join-Path $cacheRoot 'go-build')
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
    & $goExe -C (Join-Path $apkRoot 'core') test -modfile $moduleFile -mod=readonly -p 2 ./... -count=1 -v
    if ($LASTEXITCODE -ne 0) { throw 'Core tests failed' }
    & $goExe -C (Join-Path $apkRoot 'core') vet -modfile $moduleFile -mod=readonly -p 2 ./...
    if ($LASTEXITCODE -ne 0) { throw 'Core vet failed' }
    & $goExe -C (Join-Path $apkRoot 'core') mod verify
    if ($LASTEXITCODE -ne 0) { throw 'Go dependency verification failed' }
} finally {
    foreach ($name in $previous.Keys) { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
}
