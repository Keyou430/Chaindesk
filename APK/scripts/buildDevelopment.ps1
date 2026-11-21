param(
    [ValidateSet('arm64-v8a', 'x86_64')]
    [string]$Abi = 'arm64-v8a',
    [ValidateRange(4, 2147483647)][int]$VersionCode = 4
)

$ErrorActionPreference = 'Stop'
$apkRoot = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
$androidRoot = Join-Path $apkRoot 'android'
$manifest = Get-Content -LiteralPath (Join-Path $apkRoot 'toolchain.json') -Encoding UTF8 -Raw | ConvertFrom-Json
$cacheRoot = Join-Path $env:USERPROFILE '.cache/chaindesk-android'
$sdkRoot = Join-Path $cacheRoot 'sdk'
$settings = @{
    JAVA_HOME = (Join-Path $cacheRoot "unpacked/jdk-$($manifest.artifacts.jdk.version)/$($manifest.artifacts.jdk.directory)")
    ANDROID_HOME = $sdkRoot
    ANDROID_USER_HOME = (Join-Path $cacheRoot 'android-user')
    GRADLE_USER_HOME = (Join-Path $cacheRoot 'gradle-cache')
}
$previous = @{}
try {
    foreach ($name in $settings.Keys) {
        $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        [Environment]::SetEnvironmentVariable($name, $settings[$name], 'Process')
    }
    $localProperties = Join-Path $androidRoot 'local.properties'
    if (Test-Path -LiteralPath $localProperties) {
        $explicit = Select-String -LiteralPath $localProperties -Encoding UTF8 -Pattern '^\s*sdk\.dir\s*[:=]\s*(.*?)\s*$' | Select-Object -Last 1
        if ($explicit -and [IO.Path]::GetFullPath([regex]::Unescape($explicit.Matches[0].Groups[1].Value)) -ne [IO.Path]::GetFullPath($sdkRoot)) {
            throw 'Existing local.properties selects another SDK; the file was preserved'
        }
    }
    $repositoryUri = [Uri]'https://dl.google.com/dl/android/maven2/'
    $proxy = [Net.WebRequest]::GetSystemWebProxy()
    $networkOptions = @()
    if (-not $proxy.IsBypassed($repositoryUri)) {
        $proxyUri = $proxy.GetProxy($repositoryUri)
        $networkOptions = @("-Dhttps.proxyHost=$($proxyUri.Host)", "-Dhttps.proxyPort=$($proxyUri.Port)", "-Dhttp.proxyHost=$($proxyUri.Host)", "-Dhttp.proxyPort=$($proxyUri.Port)")
    }
    $staging = Join-Path $cacheRoot ('development-package-' + [guid]::NewGuid().ToString('N'))
    $source = & (Join-Path $PSScriptRoot 'createSourceArchive.ps1') -OutputDirectory $staging
    $inputHash = (Get-FileHash -LiteralPath (Join-Path $staging 'source-inputs.json') -Algorithm SHA256).Hash
    & (Join-Path $androidRoot 'gradlew.bat') -p $androidRoot --no-daemon @networkOptions "-PprobeAbi=$Abi" "-PdevelopmentVersionCode=$VersionCode" :app:clean :app:lintDebug :app:testDebugUnitTest :app:assembleDebug
    if ($LASTEXITCODE -ne 0) { throw "Development build failed with exit code $LASTEXITCODE" }
    $candidate = Join-Path $androidRoot 'app/build/outputs/apk/debug/app-debug.apk'
    $buildMetadata = Get-Content -LiteralPath (Join-Path (Split-Path $candidate -Parent) 'output-metadata.json') -Encoding UTF8 -Raw | ConvertFrom-Json
    if ($buildMetadata.applicationId -ne 'dev.chaindesk.android.debug' -or $buildMetadata.elements.Count -ne 1 -or $buildMetadata.elements[0].versionCode -ne $VersionCode) {
        throw 'Unexpected development application metadata'
    }
    $buildTools = Join-Path $sdkRoot "build-tools/$($manifest.android.buildTools)"
    & (Join-Path $buildTools 'apksigner.bat') verify --verbose --print-certs $candidate
    if ($LASTEXITCODE -ne 0) { throw 'Development APK signature verification failed' }
    & (Join-Path $buildTools 'zipalign.exe') -c -P 16 4 $candidate
    if ($LASTEXITCODE -ne 0) { throw 'Development APK alignment verification failed' }
    $readElf = Join-Path $sdkRoot "ndk/$($manifest.android.ndk)/toolchains/llvm/prebuilt/windows-x86_64/bin/llvm-readelf.exe"
    $zip = [IO.Compression.ZipFile]::OpenRead($candidate)
    try {
        if ($zip.Entries.FullName | Where-Object { $_ -match '(?i)(download-checks/|\.p12$|\.jks$|\.keystore$|fixture\.json$)' }) { throw 'APK contains lab secrets or fixtures' }
        $libraries = @($zip.Entries.FullName | Where-Object { $_ -match '^lib/[^/]+/[^/]+\.so$' })
        if ($libraries.Count -ne 3 -or "lib/$Abi/libchaindesk_core.so" -notin $libraries -or "lib/$Abi/libchaindesk_jni.so" -notin $libraries -or "lib/$Abi/libandroidx.graphics.path.so" -notin $libraries) { throw 'Unexpected packaged native ABI or library' }
        $graphicsVersionReader = [IO.StreamReader]::new($zip.GetEntry('META-INF/androidx.graphics_graphics-path.version').Open())
        try { $graphicsVersion = $graphicsVersionReader.ReadToEnd().Trim() } finally { $graphicsVersionReader.Dispose() }
        if ($graphicsVersion -ne '1.0.1') { throw 'Unexpected AndroidX graphics-path version; review its native compatibility' }
        foreach ($library in $libraries) {
            $temporaryLibrary = Join-Path $staging ([IO.Path]::GetFileName($library))
            [IO.Compression.ZipFileExtensions]::ExtractToFile($zip.GetEntry($library), $temporaryLibrary)
            $elf = & $readElf -h -lW $temporaryLibrary 2>&1
            if ($LASTEXITCODE -ne 0) { throw 'Packaged ELF inspection failed' }
            $machine = if ($Abi -eq 'arm64-v8a') { 'AArch64' } else { 'Advanced Micro Devices X86-64' }
            if (($elf -join "`n") -notmatch "Machine:\s+$([regex]::Escape($machine))") { throw 'Packaged ELF has the wrong machine' }
            $loads = @($elf | Where-Object { $_ -match '^\s*LOAD\s' })
            if ($loads.Count -eq 0) { throw 'Packaged ELF has no load segments' }
            foreach ($load in $loads) {
                if ($load -notmatch '\s0x([0-9a-fA-F]+)\s*$' -or [Convert]::ToInt64($Matches[1], 16) -lt 16384) { throw 'Packaged ELF is not aligned for 16 KiB pages' }
            }
            Set-Content -LiteralPath "$temporaryLibrary.elf.txt" -Value $elf -Encoding UTF8
            Remove-Item -LiteralPath $temporaryLibrary
        }
    } finally { $zip.Dispose() }
    $hash = (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash.ToLowerInvariant()
    $version = $buildMetadata.elements[0]
    $distribution = Join-Path $apkRoot "dist/development/v$($version.versionCode)-$Abi-$($hash.Substring(0, 12))"
    if (Test-Path -LiteralPath $distribution) { throw 'This package already has a distribution directory; existing evidence was preserved' }
    $inputCheck = Join-Path $cacheRoot ('development-input-check-' + [guid]::NewGuid().ToString('N'))
    & (Join-Path $PSScriptRoot 'createSourceArchive.ps1') -OutputDirectory $inputCheck | Out-Null
    if ((Get-FileHash -LiteralPath (Join-Path $inputCheck 'source-inputs.json') -Algorithm SHA256).Hash -ne $inputHash) { throw 'Source changed during the build; no distribution was published' }
    $packageName = "chaindesk-android-$($version.versionName)-$Abi-debug.apk"
    Copy-Item -LiteralPath $candidate -Destination (Join-Path $staging $packageName)
    [ordered]@{
        schemaVersion = 1; applicationId = $buildMetadata.applicationId; versionCode = $version.versionCode; versionName = $version.versionName
        abi = $Abi; apkSha256 = $hash; graphicsPathVersion = $graphicsVersion; sourceSha256 = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
        signature = 'Android debug identity; not a long-term release key'; acceptance = 'Development package; ARM64 and release acceptance remain open'
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $staging 'build-record.json') -Encoding UTF8
    $sums = Get-ChildItem -LiteralPath $staging -File | Sort-Object Name | ForEach-Object { "$((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant())  $($_.Name)" }
    Set-Content -LiteralPath (Join-Path $staging 'SHA256SUMS') -Value $sums -Encoding UTF8
    New-Item -ItemType Directory -Path $distribution | Out-Null
    Get-ChildItem -LiteralPath $staging -File | Copy-Item -Destination $distribution
    [ordered]@{ package = (Join-Path $distribution $packageName); sha256 = $hash; source = (Join-Path $distribution 'chaindesk-android-source.zip') } | ConvertTo-Json
} finally {
    foreach ($name in $previous.Keys) { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
}
