param(
    [Parameter(Mandatory)][string]$Serial,
    [Parameter(Mandatory)][string]$Fixture,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateRange(0, 100)][int]$LifecycleCycles = 0,
    [ValidateRange(0, 20)][int]$NetworkSwitches = 0
)

$ErrorActionPreference = 'Stop'
if ($Serial -notmatch '^emulator-\d+$') { throw 'This controlled fixture runner requires an explicitly named emulator' }
$adb = Join-Path $env:USERPROFILE '.cache/chaindesk-android/sdk/platform-tools/adb.exe'
$package = 'dev.chaindesk.android.debug'
$trafficRunner = 'dev.chaindesk.trafficprobe.debug/dev.chaindesk.trafficprobe.TrafficInstrumentation'
$fixturePath = (Resolve-Path -LiteralPath $Fixture).Path
$metadata = Get-Content -LiteralPath $fixturePath -Encoding UTF8 -Raw | ConvertFrom-Json
if ((Test-Path -LiteralPath $OutputDirectory) -and (Get-ChildItem -LiteralPath $OutputDirectory -Force | Measure-Object).Count -gt 0) { throw 'Evidence output directory must be new or empty' }
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$outputRoot = (Resolve-Path -LiteralPath $OutputDirectory).Path
$instrumentation = $null

function Invoke-Adb([string[]]$Arguments) {
    $output = & $adb -s $Serial @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) { throw "ADB command failed: $($Arguments[0])" }
    return ($output -join "`n")
}

function Get-Counts {
    return Get-Content -LiteralPath "$fixturePath.counts" -Encoding UTF8 -Raw | ConvertFrom-Json
}

function Wait-Checkpoint([string]$Name, [int]$TimeoutSeconds = 55) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $encoded = & $adb -s $Serial shell run-as $package cat cache/app-flow-state.json 2>$null
        try {
            $snapshot = ($encoded -join '') | ConvertFrom-Json
            if ($snapshot.checkpoint -eq $Name) { return $snapshot }
        } catch { }
        if ($instrumentation.HasExited) { throw "Product instrumentation ended before $Name; inspect product-instrumentation.log" }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for $Name"
}

function Send-Command([string]$Name) {
    if ($Name -notin @('releaseHealth', 'switchBad', 'cancel', 'stop', 'finishNetworks')) { throw 'Unknown checkpoint command' }
    Invoke-Adb @('shell', "run-as $package sh -c 'echo $Name > cache/app-flow-command'") | Out-Null
}

function Invoke-Traffic([string]$Name, [string]$Mode, [bool]$Vpn) {
    $targetHost = if ($Mode -eq 'udp') { '10.0.2.2' } else { $metadata.targetHost }
    $port = if ($Mode -eq 'udp') { $metadata.udpPort } else { $metadata.targetPort }
    $arguments = @('shell', 'am', 'instrument', '-w', '-e', 'mode', $Mode, '-e', 'host', $targetHost,
        '-e', 'port', "$port", '-e', 'requireVpn', $Vpn.ToString().ToLowerInvariant())
    if ($Mode -eq 'tcp') { $arguments += @('-e', 'certificate', $metadata.certificate) }
    $arguments += $trafficRunner
    $result = Invoke-Adb $arguments
    Set-Content -LiteralPath (Join-Path $outputRoot "$Name.log") -Value $result -Encoding UTF8
    return $result
}

try {
    Invoke-Adb @('shell', 'pm', 'grant', $package, 'android.permission.POST_NOTIFICATIONS') | Out-Null
    Invoke-Adb @('shell', 'pm', 'grant', $package, 'android.permission.ACCESS_LOCAL_NETWORK') | Out-Null
    Invoke-Adb @('shell', 'pm', 'grant', 'dev.chaindesk.trafficprobe.debug', 'android.permission.ACCESS_LOCAL_NETWORK') | Out-Null
    $baseline = Get-Counts
    $udpBaseline = Invoke-Traffic 'udp-without-vpn' 'udp' $false
    Start-Sleep -Milliseconds 150
    if ($udpBaseline -notmatch 'vpnActive=false' -or (Get-Counts).udpPackets -ne $baseline.udpPackets + 1) { throw 'UDP baseline was not delivered without VPN' }
    Set-Content -LiteralPath "$fixturePath.holdhealth" -Value 'hold' -Encoding UTF8
    Invoke-Adb @('shell', 'run-as', $package, 'rm', '-f', 'cache/app-flow-state.json', 'cache/app-flow-command') | Out-Null
    $encodedFixture = [Convert]::ToBase64String([IO.File]::ReadAllBytes($fixturePath))
    $instrumentation = Start-Process -FilePath $adb -WindowStyle Hidden -PassThru -ArgumentList @('-s', $Serial, 'shell', 'am', 'instrument', '-w',
        '-e', 'productFixture', $encodedFixture, '-e', 'lifecycleCycles', $LifecycleCycles, '-e', 'networkSwitches', $NetworkSwitches, "$package.test/dev.chaindesk.android.A0Instrumentation") `
        -RedirectStandardOutput (Join-Path $outputRoot 'product-instrumentation.log') -RedirectStandardError (Join-Path $outputRoot 'product-instrumentation-error.log')
    $paused = Wait-Checkpoint 'probing'
    $pausedCounts = Get-Counts
    $unverified = Invoke-Traffic 'unverified-tcp' 'tcp' $true
    Start-Sleep -Milliseconds 100
    if ($unverified -notmatch 'vpnActive=true' -or $unverified -match 'success=true' -or (Get-Counts).targetRequests -ne $pausedCounts.targetRequests) { throw 'Unverified session forwarded target traffic' }
    Remove-Item -LiteralPath "$fixturePath.holdhealth"
    Send-Command 'releaseHealth'
    $connected = Wait-Checkpoint 'connected'
    $beforeTarget = Get-Counts
    $healthy = Invoke-Traffic 'verified-tcp' 'tcp' $true
    Start-Sleep -Milliseconds 100
    $afterTarget = Get-Counts
    if ($healthy -notmatch 'success=true' -or $healthy -notmatch 'vpnActive=true' -or
        $afterTarget.targetRequests -ne $beforeTarget.targetRequests + 1 -or $afterTarget.exitTargetConnections -ne $beforeTarget.exitTargetConnections + 1 -or
        $afterTarget.frontConnections -le $beforeTarget.frontConnections) { throw 'Independent application did not traverse both controlled hops' }
    $trafficUid = [int]([regex]::Match($healthy, 'uid=(\d+)').Groups[1].Value)
    if ($trafficUid -eq $connected.uid) { throw 'Traffic probe shares the VPN application UID' }
    $udpBefore = Get-Counts
    $udpBlocked = Invoke-Traffic 'udp-with-vpn' 'udp' $true
    Start-Sleep -Milliseconds 300
    if ($udpBlocked -notmatch 'sent=true' -or (Get-Counts).udpPackets -ne $udpBefore.udpPackets) { throw 'Non-DNS UDP reached the host collector through VPN' }
    Invoke-Adb @('shell', 'screencap', '-p', '/sdcard/chaindesk-product-connected.png') | Out-Null
    Invoke-Adb @('pull', '/sdcard/chaindesk-product-connected.png', (Join-Path $outputRoot 'connected.png')) | Out-Null
    Send-Command 'switchBad'
    $rolledBack = Wait-Checkpoint 'rollbackConnected'
    $rollback = Invoke-Traffic 'rollback-tcp' 'tcp' $true
    if ($rollback -notmatch 'success=true') { throw 'Verified rollback did not restore application traffic' }
    Send-Command 'cancel'
    $blocked = Wait-Checkpoint 'wrongExitBlocked'
    $blockedCounts = Get-Counts
    $wrongExit = Invoke-Traffic 'wrong-exit-tcp' 'tcp' $true
    Start-Sleep -Milliseconds 100
    $afterBlocked = Get-Counts
    if ($wrongExit -match 'success=true' -or $afterBlocked.targetRequests -ne $blockedCounts.targetRequests -or
        $afterBlocked.rejectedAuthentication -le $baseline.rejectedAuthentication) { throw 'Wrong exit was not blocked with explicit authentication evidence' }
    Send-Command 'stop'
    if ($NetworkSwitches -gt 0) {
        $recovered = Wait-Checkpoint 'networkRecovered' (65 + $LifecycleCycles * 40 + $NetworkSwitches * 50)
        $recoveryCounts = Get-Counts
        $recoveryTraffic = Invoke-Traffic 'network-recovery-tcp' 'tcp' $true
        Start-Sleep -Milliseconds 100
        if ($recoveryTraffic -notmatch 'success=true' -or (Get-Counts).targetRequests -ne $recoveryCounts.targetRequests + 1) { throw 'Independent application traffic failed after physical network recovery' }
        Send-Command 'finishNetworks'
    }
    $finished = Wait-Checkpoint 'finished' (65 + $LifecycleCycles * 40 + $NetworkSwitches * 50)
    if (-not $instrumentation.WaitForExit(20000)) { throw 'Product instrumentation failed to stop' }
    $productResult = Get-Content -LiteralPath (Join-Path $outputRoot 'product-instrumentation.log') -Encoding UTF8 -Raw
    if ($productResult -notmatch 'passed=true' -or $productResult -notmatch 'productFlow=passed') { throw 'Product flow checks failed' }
    if ($LifecycleCycles -gt 0) {
        $sampleText = [regex]::Match($productResult, '(?m)^INSTRUMENTATION_RESULT: lifecycleSamples=(.+)$').Groups[1].Value
        if (($sampleText | ConvertFrom-Json).Count -ne $LifecycleCycles) { throw 'Lifecycle sample count mismatch' }
    }
    if ($NetworkSwitches -gt 0) {
        $switchText = [regex]::Match($productResult, '(?m)^INSTRUMENTATION_RESULT: networkSwitches=(.+)$').Groups[1].Value
        if (($switchText | ConvertFrom-Json).Count -ne $NetworkSwitches) { throw 'Network switch count mismatch' }
    }
    [ordered]@{
        schemaVersion = 1; passed = $true; serial = $Serial; vpnUid = $connected.uid; trafficUid = $trafficUid
        checkpoints = @($paused, $connected, $rolledBack, $blocked, $finished)
        initialCounts = $baseline; finalCounts = Get-Counts; lifecycleCycles = $LifecycleCycles; networkSwitches = $NetworkSwitches
        limits = @('Host-loopback front and exit share one network namespace.', 'Public health uses the declared host transport; public exit identity and real phones remain unaccepted.',
            'A local TCP handshake alone is not target forwarding; blocked checks also require zero HTTPS target-counter growth.')
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outputRoot 'execution.json') -Encoding UTF8
    Write-Output 'Product flow and independent-UID TCP/UDP checks passed.'
} finally {
    if (Test-Path -LiteralPath "$fixturePath.holdhealth") { Remove-Item -LiteralPath "$fixturePath.holdhealth" }
    if ($instrumentation -and -not $instrumentation.HasExited) {
        & $adb -s $Serial shell am force-stop $package | Out-Null
        $instrumentation.WaitForExit(10000) | Out-Null
    }
}
