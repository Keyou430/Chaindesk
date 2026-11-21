param(
    [ValidateSet('all', 'jdk', 'go', 'androidCli', 'gradle')]
    [string]$Artifact = 'all',
    [string]$CacheRoot = (Join-Path $env:USERPROFILE '.cache\chaindesk-android'),
    [switch]$InstallSdk
)

$ErrorActionPreference = 'Stop'
$manifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\toolchain.json') -Encoding UTF8 -Raw | ConvertFrom-Json
$downloads = Join-Path $CacheRoot 'downloads'
New-Item -ItemType Directory -Path $downloads -Force | Out-Null
$selected = if ($Artifact -eq 'all') { $manifest.artifacts.PSObject.Properties } else { $manifest.artifacts.PSObject.Properties | Where-Object Name -eq $Artifact }

foreach ($entry in $selected) {
    $asset = $entry.Value
    $archive = Join-Path $downloads ([Uri]::UnescapeDataString(([Uri]$asset.url).Segments[-1]))
    if (-not (Test-Path -LiteralPath $archive)) {
        Write-Output "Downloading $($entry.Name) $($asset.version)"
        Invoke-WebRequest -Uri $asset.url -OutFile "$archive.part" -TimeoutSec 1800
        $downloadHash = (Get-FileHash -LiteralPath "$archive.part" -Algorithm $asset.hashAlgorithm).Hash
        if ($downloadHash -ne $asset.hash) { throw "Checksum mismatch for $($entry.Name)" }
        Move-Item -LiteralPath "$archive.part" -Destination $archive
    }
    $actualHash = (Get-FileHash -LiteralPath $archive -Algorithm $asset.hashAlgorithm).Hash
    if ($actualHash -ne $asset.hash) { throw "Checksum mismatch for cached $($entry.Name)" }

    $unpackRoot = Join-Path $CacheRoot "unpacked\$($entry.Name)-$($asset.version)"
    $marker = Join-Path $unpackRoot '.complete'
    if (-not (Test-Path -LiteralPath $marker)) {
        New-Item -ItemType Directory -Path $unpackRoot -Force | Out-Null
        if ($asset.format -eq 'exe') {
            $binaryDirectory = Join-Path $unpackRoot $asset.directory
            New-Item -ItemType Directory -Path $binaryDirectory -Force | Out-Null
            Copy-Item -LiteralPath $archive -Destination (Join-Path $binaryDirectory 'android-cli.exe')
        }
        else { [IO.Compression.ZipFile]::ExtractToDirectory($archive, $unpackRoot, $true) }
        Set-Content -LiteralPath $marker -Value $asset.hash -Encoding UTF8
    }
    [pscustomobject]@{ Artifact = $entry.Name; Version = $asset.version; Directory = (Join-Path $unpackRoot $asset.directory); Hash = $actualHash } | ConvertTo-Json -Compress
}

if ($InstallSdk) {
    $jdk = $manifest.artifacts.jdk
    $cli = $manifest.artifacts.androidCli
    $jdkRoot = Join-Path $CacheRoot "unpacked\jdk-$($jdk.version)\$($jdk.directory)"
    $androidCli = Join-Path $CacheRoot "unpacked\androidCli-$($cli.version)\$($cli.directory)\android-cli.exe"
    $previousJavaHome = $env:JAVA_HOME
    $previousAndroidUserHome = $env:ANDROID_USER_HOME
    try {
        $env:JAVA_HOME = $jdkRoot
        $env:ANDROID_USER_HOME = Join-Path $CacheRoot 'android-user'
        $sdkRoot = Join-Path $CacheRoot 'sdk'
        $cliVersion = & $androidCli --no-metrics --version
        if ($LASTEXITCODE -ne 0 -or $cliVersion.Trim() -ne $cli.version) { throw 'Android CLI version does not match toolchain.json' }
        & $androidCli --no-metrics "--sdk=$sdkRoot" sdk install @($manifest.android.sdkPackages)
        if ($LASTEXITCODE -ne 0) { throw "SDK installation failed with exit code $LASTEXITCODE" }
        foreach ($package in $manifest.android.sdkPackages) {
            $packagePath = $package.Split('@')[0].Replace('/', [IO.Path]::DirectorySeparatorChar)
            if (-not (Test-Path -LiteralPath (Join-Path $sdkRoot "$packagePath\package.xml"))) {
                throw "SDK package was not installed: $package"
            }
        }
    }
    finally {
        $env:JAVA_HOME = $previousJavaHome
        $env:ANDROID_USER_HOME = $previousAndroidUserHome
    }
}
