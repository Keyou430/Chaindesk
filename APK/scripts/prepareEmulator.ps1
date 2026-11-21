$ErrorActionPreference = 'Stop'
$apkRoot = Split-Path $PSScriptRoot -Parent
$cacheRoot = Join-Path $env:USERPROFILE '.cache/chaindesk-android'
$sdkRoot = Join-Path $cacheRoot 'sdk'
$tools = Get-Content -LiteralPath (Join-Path $apkRoot 'toolchain.json') -Encoding UTF8 -Raw | ConvertFrom-Json
$validation = Get-Content -LiteralPath (Join-Path $apkRoot 'testFixtures/emulator-toolchain.json') -Encoding UTF8 -Raw | ConvertFrom-Json
$cli = Join-Path $cacheRoot "unpacked/androidCli-$($tools.artifacts.androidCli.version)/bin/android-cli.exe"
$previousAndroidUserHome = $env:ANDROID_USER_HOME
try {
    $env:ANDROID_USER_HOME = Join-Path $cacheRoot 'android-user'
    & $cli --no-metrics "--sdk=$sdkRoot" sdk install $validation.emulator.package $validation.image.package
    if ($LASTEXITCODE -ne 0) { throw 'Pinned emulator/image installation failed' }
} finally {
    $env:ANDROID_USER_HOME = $previousAndroidUserHome
}
$emulator = Join-Path $sdkRoot 'emulator/emulator.exe'
if ((Get-FileHash -LiteralPath $emulator -Algorithm SHA256).Hash.ToLowerInvariant() -ne $validation.emulator.executableSha256) {
    throw 'Emulator executable hash does not match the validation toolchain'
}
$imageRoot = Join-Path $sdkRoot $validation.image.directory
[xml]$imagePackage = Get-Content -LiteralPath (Join-Path $imageRoot 'package.xml') -Encoding UTF8 -Raw
$revision = $imagePackage.SelectSingleNode("//*[local-name()='localPackage']/*[local-name()='revision']/*[local-name()='major']")
if ($revision.InnerText -ne [string]$validation.image.revision) { throw 'Unexpected system image revision' }

$avdRoot = Join-Path $cacheRoot 'avd'
$avdDirectory = Join-Path $avdRoot "$($validation.avdName).avd"
New-Item -ItemType Directory -Path $avdDirectory -Force | Out-Null
$encoding = [Text.UTF8Encoding]::new($false)
$profile = @"
avd.ini.encoding=UTF-8
path=$($avdDirectory.Replace('\', '/'))
target=android-37.0
"@
$configuration = @"
AvdId=$($validation.avdName)
avd.ini.displayname=ChainDesk A0 API37 16KB
abi.type=x86_64
hw.cpu.arch=x86_64
hw.cpu.ncore=2
hw.ramSize=2048
hw.lcd.width=720
hw.lcd.height=1280
hw.lcd.density=320
hw.gpu.enabled=yes
hw.gpu.mode=swiftshader
hw.keyboard=yes
hw.battery=yes
disk.dataPartition.size=2G
image.sysdir.1=$($imageRoot.Replace('\', '/'))/
tag.id=google_apis_ps16k
tag.display=Google APIs 16KB
PlayStore.enabled=false
fastboot.forceColdBoot=yes
showDeviceFrame=no
"@
[IO.File]::WriteAllText((Join-Path $avdRoot "$($validation.avdName).ini"), $profile, $encoding)
[IO.File]::WriteAllText((Join-Path $avdDirectory 'config.ini'), $configuration, $encoding)
[pscustomobject]@{ Emulator = $emulator; AvdName = $validation.avdName; AvdHome = $avdRoot; Image = $imageRoot; Scope = $validation.scope } | ConvertTo-Json
