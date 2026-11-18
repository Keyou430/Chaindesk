if ($env:COMPUTERNAME -ne 'CHAINDESK-M0' -or $PSScriptRoot -ne 'C:\M0\M1' -or $Milestone -notin @('m3', 'm4')) {
    throw 'Recovery faults are restricted to the disposable acceptance guest'
}

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Threading;
public static class RecoveryFaultNative {
    [StructLayout(LayoutKind.Sequential)] private struct ThreadEntry {
        public uint Size, Usage, ThreadId, OwnerId; public int BasePriority, DeltaPriority; public uint Flags;
    }
    [DllImport("kernel32.dll", SetLastError=true)] private static extern IntPtr CreateToolhelp32Snapshot(uint flags, uint pid);
    [DllImport("kernel32.dll")] private static extern bool Thread32First(IntPtr snapshot, ref ThreadEntry entry);
    [DllImport("kernel32.dll")] private static extern bool Thread32Next(IntPtr snapshot, ref ThreadEntry entry);
    [DllImport("kernel32.dll")] private static extern IntPtr OpenThread(uint access, bool inherit, uint id);
    [DllImport("kernel32.dll")] private static extern uint GetProcessIdOfThread(IntPtr thread);
    [DllImport("kernel32.dll")] private static extern uint SuspendThread(IntPtr thread);
    [DllImport("kernel32.dll")] private static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);
    public static int PauseFor(uint pid, int milliseconds) {
        var handles = new List<IntPtr>();
        var snapshot = CreateToolhelp32Snapshot(4, 0);
        if (snapshot == new IntPtr(-1)) throw new InvalidOperationException("Thread snapshot failed");
        try {
            var entry = new ThreadEntry(); entry.Size = (uint)Marshal.SizeOf(entry);
            if (Thread32First(snapshot, ref entry)) do {
                if (entry.OwnerId != pid) continue;
                var thread = OpenThread(0x0802, false, entry.ThreadId);
                if (thread == IntPtr.Zero) continue;
                if (GetProcessIdOfThread(thread) == pid && SuspendThread(thread) != uint.MaxValue) handles.Add(thread);
                else CloseHandle(thread);
            } while (Thread32Next(snapshot, ref entry));
        } finally { CloseHandle(snapshot); }
        var resume = new Thread(() => { Thread.Sleep(milliseconds); foreach (var thread in handles) { ResumeThread(thread); CloseHandle(thread); } });
        resume.IsBackground = true; resume.Start();
        return handles.Count;
    }
}
'@

function Get-WorkingCore {
    return ,@(Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $application.Id -and $_.ExecutablePath -eq (Join-Path $root 'core\mihomo.exe') })
}

function Get-ProxyWatchdog {
    return ,@(Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $application.Id -and $_.ExecutablePath -eq (Join-Path $root 'ChainDesk.exe') -and $_.CommandLine -match '--proxy-watchdog [0-9a-f]{64}' })
}

function Get-ProbeRequestCount {
    $fixture = Invoke-RestMethod -Uri "$Controller/probe-fixture" -TimeoutSec 5
    return [long]($fixture.requests.PSObject.Properties.Value | Measure-Object -Sum).Sum
}

function Invoke-RecoveryEntry([string[]]$Arguments) {
    $process = Start-Process -FilePath (Join-Path $root 'ChainDesk.exe') -ArgumentList $Arguments -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $caseRoot 'recovery-stdout.log') -RedirectStandardError (Join-Path $caseRoot 'recovery-stderr.log')
    $null = $process.Handle
    if (-not $process.WaitForExit(15000)) { $process.Kill(); $process.WaitForExit(); throw 'Independent recovery entry timed out' }
    return $process.ExitCode
}

function Start-ConnectionRequest($CandidateInput) {
    $payload = @{ input = $CandidateInput } | ConvertTo-Json -Depth 10 -Compress
    [void](Invoke-DebuggerExpression "(() => { window.__chainFaultResult = null; window.__TAURI_INTERNALS__.invoke('connect_chain',$payload).then(value => window.__chainFaultResult={ok:true,value},error => window.__chainFaultResult={ok:false,code:error.code}); return true; })()")
}

function Get-ConnectionRequest {
        $diagnostic = Get-ChildItem -LiteralPath (Join-Path $data 'runtime') -Filter 'diagnostic.yaml' -Recurse -File -ErrorAction SilentlyContinue | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
        if ($diagnostic) {
            try {
                $stream = [IO.File]::Open($diagnostic.FullName, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
                $reader = [IO.StreamReader]::new($stream, [Text.Encoding]::UTF8)
                try { $captured = $reader.ReadToEnd() } finally { $reader.Dispose() }
                [IO.File]::WriteAllText((Join-Path $data 'test-diagnostic.yaml'), $captured, [Text.UTF8Encoding]::new($false))
            } catch [IO.IOException] {
                # The owned diagnostic may finish and unlink its input between polls.
            }
        }
    return Invoke-DebuggerExpression 'window.__chainFaultResult'
}

function Wait-ConnectionRequest {
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    do {
        $response = Get-ConnectionRequest
        if ($null -ne $response) { return $response }
        Start-Sleep -Milliseconds 100
    } until ([DateTime]::UtcNow -gt $deadline)
    throw 'Connection fault request did not settle'
}

$probeFixture = Set-ProbeFixtureMode 'ok'

$connectionInput = @{ frontId = $first.id; preset = 'allProxy'; probeConsent = $true; backgroundChecks = $false; expectedIpv4 = '' }
Start-ConnectionRequest $connectionInput
$response = Wait-ConnectionRequest
if (-not $response.ok) { throw "Initial recovery connection failed: $($response.code)" }
$state = $response.value
$core = Get-WorkingCore
$watchdog = Get-ProxyWatchdog
$ownedProxy = Get-ProxySnapshot
Confirm-Check ($state.phase -eq 'connected' -and $core.Count -eq 1 -and $watchdog.Count -eq 1 -and $state.canRestoreLastGood) 'M3 connects only after isolated and working checks with one working core and one recovery process'
Confirm-Check (@(Get-ChildItem -LiteralPath (Join-Path $data 'runtime') -Filter 'diagnostic.yaml' -Recurse -File).Count -eq 0) 'Completed isolated probes remove all private diagnostic plaintext configurations'
$requestsBeforeCoreExit = Get-ProbeRequestCount
Stop-Process -Id $core[0].ProcessId -Force
$deadline = [DateTime]::UtcNow.AddSeconds(15)
do { Start-Sleep -Milliseconds 200; $state = Invoke-ChainCommand 'get_chain_state' } until ($state.phase -eq 'degraded' -or [DateTime]::UtcNow -gt $deadline)
Confirm-Check ($state.phase -eq 'degraded' -and $state.error.code -eq 'CORE_EXITED' -and -not $state.backgroundChecks -and (Get-WorkingCore).Count -eq 0 -and (Get-ProxySnapshot) -eq $ownedProxy -and (Get-ProbeRequestCount) -eq $requestsBeforeCoreExit) 'A known working-core exit invalidates Connected immediately with background requests disabled and retains the owned proxy path'
Confirm-Check (@(Get-ChildItem -LiteralPath (Join-Path $data 'runtime') -Filter '*.yaml' -Recurse -File).Count -eq 0) 'A confirmed stopped working core leaves no private plaintext configuration'
Start-ConnectionRequest $connectionInput
$response = Wait-ConnectionRequest
if (-not $response.ok) { throw "Reconnect after working-core exit failed: $($response.code)" }
$state = $response.value
$core = Get-WorkingCore
Confirm-Check ($state.phase -eq 'connected' -and $core.Count -eq 1 -and (Get-ProxySnapshot) -eq $ownedProxy) 'Explicit reconnect establishes a verified working core after an opted-out lifecycle failure'
[void](Invoke-ChainCommand 'patch_verge_config' @{ payload = @{ proxy_auto_config = $true } })
$pacUrl = Get-ItemPropertyValue 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings' -Name 'AutoConfigURL'
if (([Uri]$pacUrl).Host -ne '127.0.0.1') { throw 'Owned PAC endpoint must be loopback' }
$pac = (Invoke-WebRequest -UseBasicParsing -Uri $pacUrl -TimeoutSec 5).Content
if ($pac -is [byte[]]) { $pac = [Text.Encoding]::UTF8.GetString($pac) }
$actualMixedPort = (Invoke-ChainCommand 'get_clash_info').mixed_port
$result.pacPortVerification = @{ effectivePort = $actualMixedPort; proxyTargetMatches = $pac.Contains("PROXY 127.0.0.1:$actualMixedPort;"); socksTargetMatches = $pac.Contains("SOCKS5 127.0.0.1:$actualMixedPort;"); hasDirectFallback = $pac.Contains('DIRECT') }
Confirm-Check ($result.pacPortVerification.proxyTargetMatches -and $result.pacPortVerification.socksTargetMatches -and -not $result.pacPortVerification.hasDirectFallback) 'The live portable PAC endpoint uses the actual owned fallback port for both proxy protocols without DIRECT fallback'
[void](Invoke-ChainCommand 'patch_verge_config' @{ payload = @{ proxy_auto_config = $false } })
Confirm-Check ((Get-ProxySnapshot) -eq $ownedProxy) 'Switching owned global and PAC modes preserves the same original recovery snapshot'
foreach ($unsafePatch in @(@{ pac_file_content = 'function FindProxyForURL(){return "DIRECT";}' }, @{ system_proxy_bypass = '*' }, @{ use_default_bypass = $false })) {
    $payload = @{ payload = $unsafePatch } | ConvertTo-Json -Compress
    $rejected = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('patch_verge_config',$payload).then(()=>false,()=>true)"
    Confirm-Check ($rejected -and (Get-ProxySnapshot) -eq $ownedProxy) "Unsafe legacy mutation refused without changing proxy ownership: $($unsafePatch.Keys -join ',')"
}
$settingsHash = (Get-FileHash -LiteralPath (Join-Path $data 'settings.json') -Algorithm SHA256).Hash
$credentialsHash = (Get-FileHash -LiteralPath (Join-Path $data 'credentials.bin') -Algorithm SHA256).Hash
$bad = $connectionInput.Clone()
$bad.exit = @{ host = '127.0.0.1'; port = $exitPort; username = $username; password = 'candidate-wrong-password'; remember = $false }
Start-ConnectionRequest $bad
$response = Wait-ConnectionRequest
$state = Invoke-ChainCommand 'get_chain_state'
Confirm-Check (-not $response.ok -and $state.phase -eq 'connected' -and (Get-WorkingCore)[0].ProcessId -eq $core[0].ProcessId -and (Get-ProxySnapshot) -eq $ownedProxy -and (Get-FileHash -LiteralPath (Join-Path $data 'settings.json') -Algorithm SHA256).Hash -eq $settingsHash -and (Get-FileHash -LiteralPath (Join-Path $data 'credentials.bin') -Algorithm SHA256).Hash -eq $credentialsHash) 'A rejected candidate cannot replace the working process, exit inputs, credentials or proxy ownership'
$alternate = $state.fronts | Where-Object { -not $_.rejection -and $_.id -ne $first.id } | Select-Object -First 1
$switch = $connectionInput.Clone(); $switch.frontId = $alternate.id
$result.liveSwitchCandidate = @{ name = $alternate.name; protocol = $alternate.protocol; endpoint = $alternate.endpoint }
Start-ConnectionRequest $switch
$response = Wait-ConnectionRequest
if (-not $response.ok) { throw "Live front switch failed: $($response.code)" }
$state = $response.value
Confirm-Check ($state.phase -eq 'connected' -and $state.selectedFrontId -eq $alternate.id -and $state.observation.sources.Count -eq 2 -and $state.observation.consistent) 'A live front switch passes isolated and working HTTPS checks and retains both agreeing IPv4 sources'
Start-Sleep -Seconds 3
$visibleFront = Invoke-DebuggerExpression '({selector:document.getElementById("chain-front").value,active:document.querySelector("[data-testid=chain-active-front]")?.getAttribute("data-front-id")??null})'
Confirm-Check ($visibleFront.selector -eq $state.selectedFrontId -and $visibleFront.active -eq $state.selectedFrontId) 'The home selector and verified-front display follow the backend committed choice after a live switch'
$observed = $state.observation.ipv4
$state = Invoke-ChainCommand 'confirm_chain_ipv4' @{ ipv4 = $observed }
Confirm-Check ($state.expectedIpv4 -eq $observed) 'Only an explicitly confirmed current two-source IPv4 becomes the comparison baseline'
[void](Set-ProbeFixtureMode 'changed')
$mismatch = $switch.Clone(); $mismatch.expectedIpv4 = $null
Start-ConnectionRequest $mismatch
$response = Wait-ConnectionRequest
$state = Invoke-ChainCommand 'get_chain_state'
Confirm-Check (-not $response.ok -and $response.code -eq 'EXIT_IP_CHANGED' -and $state.phase -eq 'connected' -and $state.expectedIpv4 -eq $observed -and $state.candidateObservation.ipv4 -eq '203.0.113.81') 'An actual observed IPv4 change rejects only the candidate and preserves the confirmed baseline with both source observations'
[void](Set-ProbeFixtureMode 'unavailable')
Start-ConnectionRequest $mismatch
$response = Wait-ConnectionRequest
$state = Invoke-ChainCommand 'get_chain_state'
Confirm-Check (-not $response.ok -and $response.code -eq 'EXIT_IP_UNAVAILABLE' -and $state.expectedIpv4 -eq $observed -and (Get-ProxySnapshot) -eq $ownedProxy) 'Unavailable observation services cannot silently bypass an explicitly confirmed IPv4 baseline'
[void](Set-ProbeFixtureMode 'inconsistent')
Start-ConnectionRequest $mismatch
$response = Wait-ConnectionRequest
$state = Invoke-ChainCommand 'get_chain_state'
Confirm-Check (-not $response.ok -and $response.code -eq 'EXIT_IP_INCONSISTENT' -and $state.candidateObservation.sources.Count -eq 2 -and -not $state.candidateObservation.consistent -and $state.expectedIpv4 -eq $observed) 'Disagreeing sources retain both results and cannot overwrite a confirmed baseline'
foreach ($mode in @('unavailable', 'invalid')) {
    [void](Set-ProbeFixtureMode $mode)
    $state = Invoke-ChainCommand 'connect_chain' @{ input = $connectionInput }
    Confirm-Check ($state.phase -eq 'connected' -and $null -eq $state.observation -and $state.error.code -eq 'EXIT_IP_UNAVAILABLE' -and $null -eq $state.expectedIpv4) "A verified HTTPS chain without a requested baseline remains usable with an explicit observation warning: $mode"
}
[void](Set-ProbeFixtureMode 'inconsistent')
$state = Invoke-ChainCommand 'connect_chain' @{ input = $connectionInput }
$confirm = @{ ipv4 = '203.0.113.80' } | ConvertTo-Json -Compress
$rejected = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('confirm_chain_ipv4',$confirm).then(()=>false,()=>true)"
Confirm-Check ($state.phase -eq 'connected' -and $state.error.code -eq 'EXIT_IP_INCONSISTENT' -and $state.observation.sources.Count -eq 2 -and $rejected -and $null -eq $state.expectedIpv4) 'Unconfirmed disagreement remains visible and neither source is inferred or confirmable as a baseline'
[void](Set-ProbeFixtureMode 'ok')
$automatic = $connectionInput.Clone(); $automatic.automaticFront = $true; $automatic.expectedIpv4 = $observed
$state = Invoke-ChainCommand 'connect_chain' @{ input = $automatic }
$allowed = @($state.fronts | Where-Object { -not $_.rejection } | Select-Object -First 3 | ForEach-Object { $_.id })
Confirm-Check ($state.phase -eq 'connected' -and $state.selectedFrontId -in $allowed -and (Get-WorkingCore).Count -eq 1 -and @(Get-ChildItem -LiteralPath (Join-Path $data 'runtime') -Filter 'diagnostic.yaml' -Recurse -File).Count -eq 0) 'Automatic selection is limited to three eligible fronts and leaves only the verified working core'
Start-ConnectionRequest $automatic
$deadline = [DateTime]::UtcNow.AddSeconds(5)
do { $state = Invoke-ChainCommand 'get_chain_state' } until ($state.phase -in @('probing', 'connecting') -or [DateTime]::UtcNow -gt $deadline)
[void](Invoke-ChainCommand 'cancel_chain_connection')
$response = Wait-ConnectionRequest
$state = Invoke-ChainCommand 'get_chain_state'
Confirm-Check (-not $response.ok -and $response.code -eq 'CHAIN_CANCELLED' -and $state.phase -eq 'connected' -and (Get-WorkingCore).Count -eq 1 -and @(Get-ChildItem -LiteralPath (Join-Path $data 'runtime') -Filter 'diagnostic.yaml' -Recurse -File).Count -eq 0) 'Cancellation closes only candidate jobs and retains the verified working session'
$beforeRollback = $state.selectedFrontId
$settingsBeforeRollback = (Get-FileHash -LiteralPath (Join-Path $data 'settings.json') -Algorithm SHA256).Hash
Start-ConnectionRequest $switch
$deadline = [DateTime]::UtcNow.AddSeconds(30)
do {
    $response = Get-ConnectionRequest
    if ($response -and -not $response.ok) { throw "Working fault setup failed before activation: $($response.code)" }
    $state = Invoke-ChainCommand 'get_chain_state'
} until ($state.phase -eq 'connecting' -or $response -or [DateTime]::UtcNow -gt $deadline)
Confirm-Check ($state.phase -eq 'connecting') 'Working-layer HTTPS failure is armed only after isolated candidate validation'
[void](Set-ProbeFixtureMode 'working-failure')
$response = Wait-ConnectionRequest
$state = Invoke-ChainCommand 'get_chain_state'
Confirm-Check (-not $response.ok -and $response.code -eq 'CHAIN_PROBE_FAILED' -and $state.phase -eq 'connected' -and $state.selectedFrontId -eq $beforeRollback -and (Get-FileHash -LiteralPath (Join-Path $data 'settings.json') -Algorithm SHA256).Hash -eq $settingsBeforeRollback -and (Get-ProxySnapshot) -eq $ownedProxy) 'A working-layer failure restores and rechecks the prior chain without committing the rejected candidate'

Add-Type -AssemblyName System.Security
$journalPath = Join-Path $data 'proxy-session.bin'
$encoded = [IO.File]::ReadAllBytes($journalPath)
$journal = [Text.Encoding]::UTF8.GetString([Security.Cryptography.ProtectedData]::Unprotect($encoded[4..($encoded.Length - 1)], $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)) | ConvertFrom-Json
Confirm-Check ($journal.schemaVersion -eq 1 -and $journal.phase -eq 'applied' -and $journal.parent.pid -eq $application.Id -and $journal.parent.createdAt -gt 0 -and $journal.parent.sid -eq [Security.Principal.WindowsIdentity]::GetCurrent().User.Value -and $journal.session -match '^[0-9a-f]{64}$') 'The durable current-user DPAPI journal binds exact proxy snapshots to PID, creation time, SID, executable and session'
$beforeRecovery = Get-ProxySnapshot
Confirm-Check ((Invoke-RecoveryEntry @('--recover-proxy')) -eq 1 -and (Get-ProxySnapshot) -eq $beforeRecovery) 'Independent recovery refuses a verified live owner'
Confirm-Check ((Invoke-RecoveryEntry @('--proxy-watchdog', ('0' * 64))) -eq 1 -and (Get-ProxySnapshot) -eq $ownedProxy) 'A forged watchdog session is refused without changing the owned proxy'
foreach ($attempt in 1..3) {
    $watchdog = Get-ProxyWatchdog
    Stop-Process -Id $watchdog[0].ProcessId -Force
    Start-ConnectionRequest $connectionInput
    $response = Wait-ConnectionRequest
    Start-Sleep -Seconds 7
    $state = Invoke-ChainCommand 'get_chain_state'
    Confirm-Check ($response.ok -and $state.phase -eq 'connected' -and (Get-ProxySnapshot) -eq $ownedProxy -and (Get-ProxyWatchdog).Count -eq 1) "Reconnecting after an independently killed watchdog retains one live recovery session without stale proxy restoration: $attempt"
}
$state = Invoke-ChainCommand 'set_chain_background_checks' @{ enabled = $true }
$watchdog = Get-ProxyWatchdog
Stop-Process -Id $watchdog[0].ProcessId -Force
$deadline = [DateTime]::UtcNow.AddSeconds(20)
do { Start-Sleep -Milliseconds 200; $state = Invoke-ChainCommand 'get_chain_state' } until ($state.phase -eq 'recoveryRequired' -or [DateTime]::UtcNow -gt $deadline)
Confirm-Check ($state.phase -eq 'recoveryRequired' -and (Get-ProxySnapshot) -eq $beforeProxy) 'A lost watchdog restores the original proxy and requires a new verified recovery session'
$watchdogFailedInput = $bad.Clone(); $watchdogFailedInput.backgroundChecks = $true
Start-ConnectionRequest $watchdogFailedInput
$response = Wait-ConnectionRequest
Confirm-Check (-not $response.ok) 'An invalid replacement is rejected after watchdog recovery'
$requestsAfterFailedReconnect = Get-ProbeRequestCount
$deadline = [DateTime]::UtcNow.AddSeconds(65)
do {
    Start-Sleep -Seconds 2
    $state = Invoke-ChainCommand 'get_chain_state'
    if ($state.phase -ne 'recoveryRequired') { throw 'An unowned fallback session was promoted after failed reconnect' }
} until ([DateTime]::UtcNow -gt $deadline)
Confirm-Check ($state.phase -eq 'recoveryRequired' -and (Get-ProxySnapshot) -eq $beforeProxy -and (Get-ProxyWatchdog).Count -eq 0 -and (Get-ProbeRequestCount) -eq $requestsAfterFailedReconnect) 'A failed reconnect cannot restart observation requests or report Connected without an owned proxy and recovery process'
Start-ConnectionRequest $connectionInput
$response = Wait-ConnectionRequest
if (-not $response.ok) { throw "Reconnect after watchdog recovery failed: $($response.code)" }
$state = $response.value
Confirm-Check ($state.phase -eq 'connected' -and (Get-ProxyWatchdog).Count -eq 1 -and (Get-ProxySnapshot) -eq $ownedProxy) 'Successful reconnect re-establishes the owned proxy and one authenticated recovery process'
$preview = Invoke-ChainCommand 'preview_chain_diagnostics'
$previewObject = $preview.json | ConvertFrom-Json
$sensitive = @($username, $password, $frontPassword, $uuid, 'fixture-subscription-token')
Confirm-Check (@($sensitive | Where-Object { $preview.json.Contains($_) }).Count -eq 0 -and $previewObject.observation.sources.Count -eq 2 -and $preview.sha256 -match '^[0-9a-f]{64}$') 'Diagnostic preview includes bounded observations while excluding all synthetic credentials, subscription tokens and node identities'
$diagnosticPath = Join-Path $caseRoot 'diagnostic.json'
[void](Invoke-ChainCommand 'export_chain_diagnostics' @{ path = $diagnosticPath; expectedSha256 = $preview.sha256 })
Confirm-Check ((Get-FileHash -LiteralPath $diagnosticPath -Algorithm SHA256).Hash.ToLowerInvariant() -eq $preview.sha256) 'The explicit diagnostic export matches the reviewed snapshot byte-for-byte'

$state = Invoke-ChainCommand 'disconnect_chain'
$dataDirectory = [IO.DirectoryInfo]::new($data)
$accessSection = [Security.AccessControl.AccessControlSections]::Access
$security = $dataDirectory.GetAccessControl($accessSection)
$savedSddl = $security.GetSecurityDescriptorSddlForm($accessSection)
$deny = [Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.WindowsIdentity]::GetCurrent().User, [Security.AccessControl.FileSystemRights]::CreateFiles, [Security.AccessControl.InheritanceFlags]::None, [Security.AccessControl.PropagationFlags]::None, [Security.AccessControl.AccessControlType]::Deny)
$security.AddAccessRule($deny)
try {
    $dataDirectory.SetAccessControl($security)
    Start-ConnectionRequest $connectionInput
    $response = Wait-ConnectionRequest
    Confirm-Check (-not $response.ok -and (Get-ProxySnapshot) -eq $beforeProxy -and -not (Test-Path -LiteralPath $journalPath)) 'A durable journal creation failure prevents every system proxy write'
} finally {
    $security.SetSecurityDescriptorSddlForm($savedSddl, $accessSection)
    $dataDirectory.SetAccessControl($security)
}
$state = Invoke-ChainCommand 'restore_chain_last_good'
Confirm-Check ($state.canRestoreLastGood -and $state.hasSessionAuthentication -and -not $state.probeConsent) 'Last working inputs survive failed candidates and require explicit probe consent after loading'
$state = Invoke-ChainCommand 'connect_chain' @{ input = $connectionInput }
$core = Get-WorkingCore
$watchdog = Get-ProxyWatchdog
$stoppedPlaintextPaths = @(Get-ChildItem -LiteralPath (Join-Path $data 'runtime') -Filter '*.yaml' -Recurse -File | ForEach-Object { $_.FullName })
if ($stoppedPlaintextPaths.Count -eq 0) { throw 'The crash scenario has no owned runtime plaintext to verify' }
$application.Kill(); $application.WaitForExit(); $socket.Dispose(); $socket = $null
$deadline = [DateTime]::UtcNow.AddSeconds(20)
do { Start-Sleep -Milliseconds 100 } until ((Get-ProxySnapshot) -eq $beforeProxy -or [DateTime]::UtcNow -gt $deadline)
Confirm-Check ((Get-ProxySnapshot) -eq $beforeProxy -and -not (Get-Process -Id $core[0].ProcessId -ErrorAction SilentlyContinue) -and -not (Test-Path -LiteralPath $journalPath) -and (Get-Process -Id $originalCoreId -ErrorAction SilentlyContinue)) 'A forced parent exit closes its core job and the verified watchdog restores only the owned proxy'
Start-Product $root
Confirm-Check (@($stoppedPlaintextPaths | Where-Object { Test-Path -LiteralPath $_ }).Count -eq 0 -and (Get-WorkingCore).Count -eq 1) 'Restart removes plaintext from verified stopped sessions while preserving the new live core'
$state = Invoke-ChainCommand 'connect_chain' @{ input = $connectionInput }
$watchdog = Get-ProxyWatchdog
$runtime = Join-Path $root 'runtime'
$runtimeBrowsers = @(Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" | Where-Object { $_.ExecutablePath -eq (Join-Path $runtime 'msedgewebview2.exe') } | ForEach-Object {
    $browser = Get-Process -Id $_.ProcessId -ErrorAction Stop
    $null = $browser.Handle
    $browser
})
[void][RecoveryFaultNative]::PauseFor($application.Id, 12000)
Stop-Process -Id $watchdog[0].ProcessId -Force
$application.Kill(); $application.WaitForExit(); $socket.Dispose(); $socket = $null
Confirm-Check ((Test-Path -LiteralPath $journalPath) -and (Get-ProxySnapshot) -ne $beforeProxy) 'Dual termination leaves an unfinished durable proxy session for independent recovery'
$unavailableRuntime = Join-Path $root 'runtime-unavailable'
if (-not ([IO.Path]::GetFullPath($root)).StartsWith(([IO.Path]::GetFullPath($caseRoot) + '\'), [StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($runtime)) -ne [IO.Path]::GetFullPath($root) -or [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($unavailableRuntime)) -ne [IO.Path]::GetFullPath($root)) { throw 'Runtime move escaped the owned disposable case' }
$browserExitStarted = [DateTime]::UtcNow
$deadline = $browserExitStarted.AddSeconds(15)
foreach ($browser in $runtimeBrowsers) {
    $remaining = [int][Math]::Max(1, ($deadline - [DateTime]::UtcNow).TotalMilliseconds)
    if (-not $browser.WaitForExit($remaining)) { throw 'An owned WebView2 process did not release the runtime after parent termination' }
}
$result.runtimeReleaseEvidence = @{ ownedBrowserCount = $runtimeBrowsers.Count; exitWaitMilliseconds = ([DateTime]::UtcNow - $browserExitStarted).TotalMilliseconds }
Confirm-Check (@($runtimeBrowsers | Where-Object { -not $_.HasExited }).Count -eq 0) 'Owned WebView2 processes exit naturally before the independent-recovery fixture removes their runtime'
Move-Item -LiteralPath $runtime -Destination $unavailableRuntime
try {
    Confirm-Check ((Invoke-RecoveryEntry @('--recover-proxy')) -eq 0 -and (Get-ProxySnapshot) -eq $beforeProxy -and -not (Test-Path -LiteralPath $journalPath)) 'Independent recovery works after both processes are killed without an available WebView2 runtime'
} finally { Move-Item -LiteralPath $unavailableRuntime -Destination $runtime }
Start-Product $root
$state = Invoke-ChainCommand 'connect_chain' @{ input = $connectionInput }
$watchdog = Get-ProxyWatchdog
[void][RecoveryFaultNative]::PauseFor($application.Id, 12000)
Stop-Process -Id $watchdog[0].ProcessId -Force
$application.Kill(); $application.WaitForExit(); $socket.Dispose(); $socket = $null
Start-Product $root
$state = Invoke-ChainCommand 'get_chain_state'
Confirm-Check ((Get-ProxySnapshot) -eq $beforeProxy -and -not (Test-Path -LiteralPath $journalPath) -and $state.phase -ne 'connected') 'Next startup recovers an unfinished dual-termination session without automatically reconnecting'
$state = Invoke-ChainCommand 'connect_chain' @{ input = $connectionInput }
Confirm-Check ((Get-OriginalFingerprint) -eq $originalFingerprint) 'Recovery faults preserve original-client files before the explicit foreign-client proxy mutation'
Set-Proxy $originalSocket $true
$foreign = Get-ProxySnapshot
$foreignOriginalFingerprint = Get-OriginalFingerprint
$result.foreignProxyFixtureEvidence = @{ originalConfigChangedByInjector = $foreignOriginalFingerprint -ne $originalFingerprint }
$application.Kill(); $application.WaitForExit(); $socket.Dispose(); $socket = $null
Start-Sleep -Seconds 2
Start-Product $root
$state = Invoke-ChainCommand 'get_chain_state'
Confirm-Check ($state.error.code -eq 'RESTORE_CONFLICT' -and (Get-ProxySnapshot) -eq $foreign) 'Crash recovery preserves externally changed proxy values and presents a recovery conflict'
$state = Invoke-ChainCommand 'keep_chain_current_proxy'
Confirm-Check ($state.phase -eq 'disconnected' -and (Get-ProxySnapshot) -eq $foreign -and (Get-OriginalFingerprint) -eq $foreignOriginalFingerprint -and -not (Test-Path -LiteralPath $journalPath)) 'Acknowledging a conflict keeps every current external proxy value and original-client file'
Set-Proxy $originalSocket $false
$nextProxy = Get-ProxySnapshot
$result.foreignProxyFixtureEvidence.rawProxyChangedByInjector = $nextProxy -ne $beforeProxy
$beforeProxy = $nextProxy
$originalFingerprint = Get-OriginalFingerprint
$proxy = $beforeProxy | ConvertFrom-Json
Confirm-Check ($proxy.ProxyEnable.value -eq 0 -and -not $proxy.AutoConfigURL.value -and (Get-Process -Id $originalCoreId -ErrorAction SilentlyContinue)) 'Explicitly disabling the original client establishes the next off-proxy baseline while preserving its core'
$state = Invoke-ChainCommand 'connect_chain' @{ input = $connectionInput }
Start-Sleep -Seconds 3
$idleTraffic = Get-FixtureTraffic $exitFixture
$idleProbes = Get-ProbeRequestCount
Start-Sleep -Seconds 65
$afterIdleTraffic = Get-FixtureTraffic $exitFixture
$afterIdleProbes = Get-ProbeRequestCount
$result.backgroundOptOutEvidence = @{ probeRequestsBefore = $idleProbes; probeRequestsAfter = $afterIdleProbes; otherProxyTrafficBytes = $afterIdleTraffic - $idleTraffic }
Confirm-Check ($afterIdleProbes -eq $idleProbes) 'Disabling background checks produces no scheduled observation-service requests across a full check interval independently of other applications using the proxy'
$state = Invoke-ChainCommand 'set_chain_background_checks' @{ enabled = $true }
$checkedBefore = $state.healthCheckedAt
$deadline = [DateTime]::UtcNow.AddSeconds(90)
do { Start-Sleep -Seconds 3; $state = Invoke-ChainCommand 'get_chain_state' } until ($state.healthCheckedAt -ne $checkedBefore -or [DateTime]::UtcNow -gt $deadline)
Confirm-Check ($state.phase -eq 'connected' -and $state.healthCheckedAt -ne $checkedBefore) 'Opted-in background HTTPS checks refresh working-chain health after 60 seconds independently of the 300-second IPv4 interval'
$ownedProxy = Get-ProxySnapshot
$failureStarted = [DateTime]::UtcNow
$exitFixture.process.Kill(); $exitFixture.process.WaitForExit()
$deadline = [DateTime]::UtcNow.AddSeconds(240)
do { Start-Sleep -Seconds 3; $state = Invoke-ChainCommand 'get_chain_state' } until ($state.phase -eq 'degraded' -or [DateTime]::UtcNow -gt $deadline)
$info = Invoke-ChainCommand 'get_clash_info'
& (Join-Path $env:SystemRoot 'System32\curl.exe') --noproxy never-match.invalid --connect-timeout 2 --max-time 4 --silent --fail --proxy "http://127.0.0.1:$($info.mixed_port)" https://www.gstatic.com/generate_204 > $null
$failedThroughProxy = $LASTEXITCODE -ne 0
$result.backgroundDegradedAfterSeconds = ([DateTime]::UtcNow - $failureStarted).TotalSeconds
Confirm-Check ($state.phase -eq 'degraded' -and $result.backgroundDegradedAfterSeconds -ge 120 -and $failedThroughProxy -and (Get-ProxySnapshot) -eq $ownedProxy -and (Get-WorkingCore).Count -eq 1) 'Three failed background checks mark degradation while keeping the fixed-exit proxy path without DIRECT fallback'
$exitFixture = Start-FixtureCore 'exit-recovered' @{ listeners = @(@{ name = 'fixed-exit'; type = 'socks'; listen = '127.0.0.1'; port = $exitPort; udp = $false; users = @(@{ username = $username; password = $password }) }) }
Enable-FixtureProbeRoutes 'exit-recovered' $exitFixture $probeFixture.proxyPort
$deadline = [DateTime]::UtcNow.AddSeconds(90)
do { Start-Sleep -Seconds 3; $state = Invoke-ChainCommand 'get_chain_state' } until ($state.phase -eq 'connected' -or [DateTime]::UtcNow -gt $deadline)
Confirm-Check ($state.phase -eq 'connected' -and (Get-ProxySnapshot) -eq $ownedProxy -and (Get-FixtureTraffic $exitFixture) -gt 0) 'Recovery of the same fixed exit returns to connected only after a new working-chain observation'
$autoBackground = $connectionInput.Clone(); $autoBackground.automaticFront = $true; $autoBackground.backgroundChecks = $true
$state = Invoke-ChainCommand 'connect_chain' @{ input = $autoBackground }
$failedFront = $state.selectedFrontId
$failedFrontPort = [int][regex]::Match(($state.fronts | Where-Object id -eq $failedFront).endpoint, ':(\d+)$').Groups[1].Value
$originalFixtureJson = [IO.File]::ReadAllText((Join-Path $caseRoot 'front-fixture\fixture.json'))
$modifiedFixture = $originalFixtureJson | ConvertFrom-Json
$modifiedFixture.listeners = @($modifiedFixture.listeners | Where-Object { $_.port -ne $failedFrontPort })
$fixtureControl = "http://127.0.0.1:$($frontFixture.port)/configs?force=true"
$fixtureHeaders = @{ Authorization = "Bearer $($frontFixture.token)" }
$disabledAt = [DateTime]::UtcNow
try {
    [void](Invoke-RestMethod -Method Put -Uri $fixtureControl -Headers $fixtureHeaders -ContentType 'application/json' -Body (@{ payload = ($modifiedFixture | ConvertTo-Json -Depth 30) } | ConvertTo-Json -Depth 32) -TimeoutSec 10)
    $deadline = [DateTime]::UtcNow.AddSeconds(240)
    do { Start-Sleep -Seconds 3; $state = Invoke-ChainCommand 'get_chain_state' } until (($state.phase -eq 'connected' -and $state.selectedFrontId -ne $failedFront) -or [DateTime]::UtcNow -gt $deadline)
    $result.automaticRepairAfterSeconds = ([DateTime]::UtcNow - $disabledAt).TotalSeconds
    Confirm-Check ($state.phase -eq 'connected' -and $state.selectedFrontId -ne $failedFront -and $state.automaticFront -and $state.port -eq $exitPort -and $state.host -eq '127.0.0.1' -and $result.automaticRepairAfterSeconds -ge 120) 'Automatic mode replaces a failed front only after isolated and working rechecks while preserving the exact fixed exit'
    Start-Sleep -Seconds 3
    $visibleFront = Invoke-DebuggerExpression '({selector:document.getElementById("chain-front").value,active:document.querySelector("[data-testid=chain-active-front]")?.getAttribute("data-front-id")??null})'
    Confirm-Check ($visibleFront.selector -eq $state.selectedFrontId -and $visibleFront.active -eq $state.selectedFrontId) 'Automatic background repair updates the displayed verified front and next-connection selection'
} finally {
    [void](Invoke-RestMethod -Method Put -Uri $fixtureControl -Headers $fixtureHeaders -ContentType 'application/json' -Body (@{ payload = $originalFixtureJson } | ConvertTo-Json -Depth 32) -TimeoutSec 10)
}
[void](Invoke-ChainCommand 'set_chain_background_checks' @{ enabled = $false })
$state = Invoke-ChainCommand 'disconnect_chain'
$result.recoveryPhase = $state.phase
