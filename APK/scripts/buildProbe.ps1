param(
    [ValidateSet('arm64-v8a', 'x86_64')]
    [string]$Abi = 'arm64-v8a'
)

$ErrorActionPreference = 'Stop'
$apkRoot = Split-Path $PSScriptRoot -Parent
$version = Select-String -LiteralPath (Join-Path $apkRoot 'android/app/build.gradle.kts') -Pattern '^\s*versionCode\s*=\s*(\d+)\s*$'
if ($version.Count -ne 1 -or [int]$version.Matches[0].Groups[1].Value -gt 3) {
    throw 'Current source is beyond A0. Use buildDevelopment.ps1; sealed A0 packages must be preserved.'
}
$manifest = Get-Content -LiteralPath (Join-Path $apkRoot 'toolchain.json') -Encoding UTF8 -Raw | ConvertFrom-Json
$androidCache = Join-Path $env:USERPROFILE '.cache\chaindesk-android'
$sdkRoot = Join-Path $androidCache 'sdk'
$jdk = $manifest.artifacts.jdk
$jdkRoot = Join-Path $androidCache "unpacked\jdk-$($jdk.version)\$($jdk.directory)"
$androidRoot = Join-Path $apkRoot 'android'
$settings = @{
    JAVA_HOME = $jdkRoot
    ANDROID_HOME = $sdkRoot
    ANDROID_USER_HOME = (Join-Path $androidCache 'android-user')
    GRADLE_USER_HOME = (Join-Path $androidCache 'gradle-cache')
}
$previous = @{}

try {
    foreach ($name in $settings.Keys) {
        $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        [Environment]::SetEnvironmentVariable($name, $settings[$name], 'Process')
    }
    $localProperties = Join-Path $androidRoot 'local.properties'
    if (Test-Path -LiteralPath $localProperties) {
        $explicitSdk = Get-Content -LiteralPath $localProperties -Encoding UTF8 | Select-String '^\s*sdk\.dir\s*[:=]\s*(.*?)\s*$' | Select-Object -Last 1
        if ($explicitSdk) {
            $configuredSdk = [regex]::Unescape($explicitSdk.Matches[0].Groups[1].Value)
            if ([IO.Path]::GetFullPath($configuredSdk) -ne [IO.Path]::GetFullPath($sdkRoot)) { throw 'Existing local.properties selects a different SDK; the file was preserved' }
        }
    }
    $repositoryUri = [Uri]'https://dl.google.com/dl/android/maven2/'
    $proxy = [Net.WebRequest]::GetSystemWebProxy()
    $networkOptions = @()
    if (-not $proxy.IsBypassed($repositoryUri)) {
        $proxyUri = $proxy.GetProxy($repositoryUri)
        $networkOptions = @("-Dhttps.proxyHost=$($proxyUri.Host)", "-Dhttps.proxyPort=$($proxyUri.Port)", "-Dhttp.proxyHost=$($proxyUri.Host)", "-Dhttp.proxyPort=$($proxyUri.Port)")
    }
    & (Join-Path $androidRoot 'gradlew.bat') -p $androidRoot --no-daemon @networkOptions "-PprobeAbi=$Abi" :app:clean :app:lintDebug :app:assembleDebug
    if ($LASTEXITCODE -ne 0) { throw "Android probe build failed with exit code $LASTEXITCODE" }

    $candidate = Join-Path $androidRoot 'app\build\outputs\apk\debug\app-debug.apk'
    $buildTools = Join-Path $sdkRoot "build-tools\$($manifest.android.buildTools)"
    & (Join-Path $buildTools 'apksigner.bat') verify --verbose --print-certs $candidate
    if ($LASTEXITCODE -ne 0) { throw 'Probe APK signature verification failed' }
    & (Join-Path $buildTools 'zipalign.exe') -c -P 16 -v 4 $candidate
    if ($LASTEXITCODE -ne 0) { throw 'Probe APK alignment verification failed' }
    $hash = (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash.ToLowerInvariant()
    $distribution = Join-Path $apkRoot $(if ($Abi -eq 'arm64-v8a') { 'dist\a0-runtime' } else { 'dist\a0-emulator' })
    New-Item -ItemType Directory -Path $distribution -Force | Out-Null
    $package = Join-Path $distribution "chaindesk-android-a0-$Abi-debug.apk"
    Copy-Item -LiteralPath $candidate -Destination $package
    Set-Content -LiteralPath (Join-Path $distribution 'SHA256SUMS') -Value "$hash  $([IO.Path]::GetFileName($package))" -Encoding UTF8
    [pscustomobject]@{ Package = $package; SHA256 = $hash; Scope = 'A0 VPN runtime development probe; no device acceptance' } | ConvertTo-Json
}
finally {
    foreach ($name in $previous.Keys) { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
}
