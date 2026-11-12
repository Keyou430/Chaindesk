param([Parameter(Mandatory = $true)][string]$Controller)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$machine = Get-CimInstance Win32_ComputerSystem
if ($env:COMPUTERNAME -ne 'CHAINDESK-M0' -or $machine.Manufacturer -notmatch 'QEMU' -or $PSScriptRoot -ne 'C:\M0\M1') {
    throw 'This script requires the disposable ChainDesk acceptance guest'
}
$endpoint = [Uri]$Controller
if ($endpoint.Scheme -ne 'http' -or $endpoint.Host -ne '10.0.2.2' -or $endpoint.Port -lt 1024) { throw 'Invalid guest controller' }
. (Join-Path $PSScriptRoot 'vm-debugger.ps1')
$requestId = 0
$result = [ordered]@{ passed = $false; milestone = 'M1'; checks = @() }
$result.harnessSha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
$original = $null
$application = $null
$originalSocket = $null
$socket = $null
$portRaceJob = $null
$caseRoot = Join-Path $PSScriptRoot ('case-' + [Guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $caseRoot | Out-Null
$result.caseDirectory = $caseRoot

function Confirm-Check([bool]$Condition, [string]$Name) {
    if (-not $Condition) { throw $Name }
    $script:result.checks += $Name
    $progress = @{ phase = $Name; checks = $script:result.checks.Count } | ConvertTo-Json -Compress
    Invoke-WebRequest -UseBasicParsing -Method Post -Uri "$Controller/progress" -ContentType 'application/json' -Body $progress -TimeoutSec 10 | Out-Null
}

function Connect-Application([Diagnostics.Process]$Process, [int]$Port) {
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    $debugger = $null
    do {
        if ($Process.HasExited) { throw "Application exited during startup: $($Process.ExitCode)" }
        if (-not $debugger) {
            try { $pages = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/json/list" -TimeoutSec 2 } catch { $pages = @() }
            $page = $pages | Where-Object { $_.type -eq 'page' -and $_.webSocketDebuggerUrl } | Select-Object -First 1
            if ($page) {
                $debugger = [Net.WebSockets.ClientWebSocket]::new()
                $timeout = [Threading.CancellationTokenSource]::new(10000)
                try { [void]$debugger.ConnectAsync([Uri]$page.webSocketDebuggerUrl, $timeout.Token).GetAwaiter().GetResult() } finally { $timeout.Dispose() }
            }
        }
        if ($debugger) {
            $ready = Invoke-DebuggerExpression "typeof window.__TAURI_INTERNALS__?.invoke === 'function' && (document.getElementById('root')?.childElementCount ?? 0) > 0" $debugger
            if ($ready) {
                $state = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('get_runtime_state')" $debugger
                if ($state.mode -eq 'Sidecar') { return $debugger }
            }
        }
        Start-Sleep -Milliseconds 500
    } until ([DateTime]::UtcNow -gt $deadline)
    if ($debugger) { $debugger.Dispose() }
    throw ('Application did not reach Sidecar mode: ' + ($state | ConvertTo-Json -Depth 5 -Compress))
}

function Get-ProxySnapshot {
    $key = Get-Item 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings'
    $snapshot = [ordered]@{}
    foreach ($name in @('ProxyEnable', 'ProxyServer', 'ProxyOverride', 'AutoConfigURL')) {
        $present = $key.GetValueNames() -contains $name
        $snapshot[$name] = if ($present) { @{ kind = [string]$key.GetValueKind($name); value = $key.GetValue($name) } } else { $null }
    }
    $key.Close()
    return ($snapshot | ConvertTo-Json -Depth 4 -Compress)
}

function Get-OriginalFingerprint {
    $files = @('config.yaml', 'verge.yaml', 'profiles.yaml') | ForEach-Object {
        $file = Join-Path $script:originalData $_
        if (Test-Path -LiteralPath $file) { $_ + ':' + (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash }
    }
    return ($files -join '|')
}

function Get-ProtocolFingerprint {
    $values = foreach ($name in @('clash', 'clash-verge')) {
        $path = "HKCU:\Software\Classes\$name"
        if (Test-Path $path) {
            foreach ($item in @((Get-Item $path)) + @(Get-ChildItem $path -Recurse)) {
                $entry = [ordered]@{ path = $item.Name }
                foreach ($value in ($item.GetValueNames() | Sort-Object)) { $entry[$value] = $item.GetValue($value) }
                $entry
            }
        }
    }
    return ($values | ConvertTo-Json -Depth 4 -Compress)
}

function Set-Proxy([Net.WebSockets.ClientWebSocket]$Debugger, [bool]$Enabled) {
    $value = $Enabled.ToString().ToLowerInvariant()
    [void](Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('patch_verge_config', {payload:{enable_system_proxy:$value}})" $Debugger)
}

function Stop-Application([Diagnostics.Process]$Process, [Net.WebSockets.ClientWebSocket]$Debugger) {
    [void](Invoke-DebuggerExpression "setTimeout(() => window.__TAURI_INTERNALS__.invoke('exit_app'), 100); true" $Debugger)
    if (-not $Process.WaitForExit(30000)) { throw 'Normal application exit timed out' }
    Confirm-Check ($Process.ExitCode -eq 0) 'Normal exit completed successfully'
    $Debugger.Dispose()
}

function Start-Product([string]$Root) {
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9223'
    $env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER = 'C:\M0\nonexistent-inherited-runtime'
    $env:WEBVIEW2_USER_DATA_FOLDER = 'C:\M0\forbidden-inherited-browser-data'
    $env:CLASH_HOME_DIR = 'C:\M0\forbidden-inherited-core-data'
    $env:HTTP_PROXY = 'http://127.0.0.1:1'
    $env:HTTPS_PROXY = 'http://127.0.0.1:1'
    $script:application = Start-Process -FilePath (Join-Path $Root 'ChainDesk.exe') -PassThru
    Remove-Item Env:HTTP_PROXY, Env:HTTPS_PROXY, Env:CLASH_HOME_DIR -ErrorAction SilentlyContinue
    $script:socket = Connect-Application $script:application 9223
}

function Confirm-StartupRefusal([string]$Executable, [string]$ExpectedMessage) {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class StartupDialogNative {
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessageW(IntPtr window, uint message, UIntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
}
'@
    $process = Start-Process -FilePath $Executable -PassThru
    try {
        $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty, $process.Id)
        $deadline = [DateTime]::UtcNow.AddSeconds(30)
        do {
            $elements = [Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Descendants, $condition)
            $message = ($elements | ForEach-Object { $_.Current.Name }) -join ' '
            if ($message -match $ExpectedMessage) { break }
            Start-Sleep -Milliseconds 500
        } until ($process.HasExited -or [DateTime]::UtcNow -ge $deadline)
        Confirm-Check ($message -match $ExpectedMessage) 'Unwritable location shows actionable startup error'
        $dialog = $elements | Where-Object { $_.Current.ControlType -eq [Windows.Automation.ControlType]::Window -and $_.Current.Name -eq 'ChainDesk startup failed' } | Select-Object -First 1
        if (-not $dialog) { throw 'Could not locate the startup refusal task dialog' }
        $window = [IntPtr]$dialog.Current.NativeWindowHandle
        $dialogProcessId = [uint32]0
        [void][StartupDialogNative]::GetWindowThreadProcessId($window, [ref]$dialogProcessId)
        if ($dialogProcessId -ne $process.Id) { throw 'Startup dialog belongs to another process' }
        # RFD uses TaskDialogIndirect; TDM_CLICK_BUTTON acknowledges this HWND without foreground focus.
        $posted = [StartupDialogNative]::PostMessageW($window, 1126, [UIntPtr]::new([uint64]1), [IntPtr]::Zero)
        if (-not $posted) { throw 'Could not acknowledge the owned startup refusal dialog' }
        $exited = $process.WaitForExit(10000)
        $script:result.startupRefusal = @{ exited = $exited; exitCode = if ($exited) { $process.ExitCode } else { $null }; dialogOwned = $true; acknowledgementPosted = $posted }
        Confirm-Check ($exited -and $process.ExitCode -ne 0) 'Invalid portable location exits with failure'
    } finally { if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit() } }
}

try {
    $identity = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
    Confirm-Check (-not $identity.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) 'Standard user token'
    $windows = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
    $result.windows = @{ build = [int]$windows.CurrentBuildNumber; revision = $windows.UBR; architecture = $env:PROCESSOR_ARCHITECTURE }
    $servicesBefore = @(Get-Service | Where-Object { $_.Name -match 'clash|mihomo|verge' }).Count
    Confirm-Check ($servicesBefore -eq 0) 'No proxy services installed'
    $initialProxy = Get-ProxySnapshot
    $manifest = Invoke-RestMethod -Uri "$Controller/manifest.json" -TimeoutSec 30
    $archive = Join-Path $caseRoot 'payload.zip'
    & curl.exe --fail --silent --show-error --max-time 300 --output $archive "$Controller/payload.zip"
    Confirm-Check ($LASTEXITCODE -eq 0) 'Payload downloaded'
    $result.downloadedArchiveSha256 = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    $result.downloadedArchiveBytes = (Get-Item -LiteralPath $archive).Length
    Confirm-Check ($result.downloadedArchiveSha256 -eq $manifest.archiveSha256) 'ZIP checksum verified'
    $workspace = Join-Path $caseRoot ('portable ' + [char]0x4fbf + [char]0x643a)
    New-Item -ItemType Directory -Path $workspace | Out-Null
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [IO.Compression.ZipFile]::ExtractToDirectory($archive, $workspace)
    Confirm-Check ($true) 'Payload extracted'
    foreach ($file in $manifest.files) {
        $target = [IO.Path]::GetFullPath((Join-Path $workspace $file.path))
        if (-not $target.StartsWith($workspace + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Payload path escapes workspace' }
        if ((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash.ToLowerInvariant() -ne $file.sha256) { throw "Payload checksum mismatch: $($file.path)" }
    }
    $result.payloadSha256 = $manifest.archiveSha256
    $result.sourceFingerprint = $manifest.sourceFingerprint
    $result.sourceCommit = $manifest.sourceCommit
    $root = Join-Path $workspace 'ChainDesk'
    $result.applicationSha256 = (Get-FileHash -LiteralPath (Join-Path $root 'ChainDesk.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
    Confirm-Check (-not (Test-Path -LiteralPath (Join-Path $root 'data'))) 'ZIP contains no user data'
    $originalData = Join-Path $env:APPDATA 'io.github.clash-verge-rev.clash-verge-rev'
    $fixtureRoot = Join-Path $caseRoot 'original app'
    New-Item -ItemType Directory -Path $fixtureRoot | Out-Null
    & curl.exe --fail --silent --show-error --max-time 180 --output (Join-Path $fixtureRoot 'clash-verge.exe') "$Controller/original.exe"
    Confirm-Check ($LASTEXITCODE -eq 0 -and (Get-FileHash -LiteralPath (Join-Path $fixtureRoot 'clash-verge.exe') -Algorithm SHA256).Hash.ToLowerInvariant() -eq $manifest.originalApplicationSha256) 'Original executable matches sealed M0 binary'
    Copy-Item -LiteralPath (Join-Path $root 'core\mihomo.exe') -Destination (Join-Path $fixtureRoot 'verge-mihomo.exe')
    Copy-Item -LiteralPath (Join-Path $root 'runtime') -Destination (Join-Path $fixtureRoot 'runtime') -Recurse
    Copy-Item -LiteralPath (Join-Path $root 'resources\rules') -Destination (Join-Path $fixtureRoot 'resources') -Recurse
    $fixtureBackup = Join-Path $caseRoot 'data-before-fixture'
    if (Test-Path -LiteralPath $originalData) { Move-Item -LiteralPath $originalData -Destination $fixtureBackup }
    New-Item -ItemType Directory -Path $originalData | Out-Null
    $fixtureSettings = Join-Path $originalData 'verge.yaml'
    [IO.File]::WriteAllText($fixtureSettings, "auto_check_update: false`nclash_core: verge-mihomo`nenable_global_hotkey: false`n", [Text.UTF8Encoding]::new($false))
    $result.originalUpdaterDisabledByFixture = $true
    $result.originalDataRebuiltByFixture = $true
    $env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER = Join-Path $fixtureRoot 'runtime'
    $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $fixtureRoot 'browser-data'
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
    $original = Start-Process -FilePath (Join-Path $fixtureRoot 'clash-verge.exe') -PassThru
    $originalSocket = Connect-Application $original 9222
    $protocolsBefore = Get-ProtocolFingerprint
    $originalCore = @(Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $original.Id -and $_.Name -eq 'verge-mihomo.exe' })
    Confirm-Check ($originalCore.Count -eq 1) 'Original client owns its separate core'
    $originalCoreId = $originalCore[0].ProcessId
    $originalPort = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('get_clash_info').then(x => x.mixed_port)" $originalSocket
    $originalFingerprint = Get-OriginalFingerprint
    $raceReady = Join-Path $caseRoot 'port-race-ready'
    $raceBound = Join-Path $caseRoot 'port-race-bound.json'
    $portRaceJob = Start-Job -ArgumentList $root, $raceReady, $raceBound -ScriptBlock {
        param($ProductRoot, $ReadyFile, $BoundFile)
        [IO.File]::WriteAllText($ReadyFile, 'ready')
        $deadline = [DateTime]::UtcNow.AddMinutes(3)
        do {
            $config = Get-ChildItem -LiteralPath (Join-Path $ProductRoot 'data\runtime') -Filter 'clash-verge.yaml' -Recurse -File -ErrorAction SilentlyContinue | Select-Object -First 1
            if ($config) {
                try {
                    $port = [int][regex]::Match([IO.File]::ReadAllText($config.FullName), '(?m)^mixed-port:\s*(\d+)').Groups[1].Value
                    if ($port -gt 0) {
                        $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, $port)
                        $listener.ExclusiveAddressUse = $true
                        $listener.Start()
                        [IO.File]::WriteAllText($BoundFile, (@{ port = $port; processId = $PID } | ConvertTo-Json -Compress))
                        try { while ($true) { Start-Sleep -Seconds 1 } } finally { $listener.Stop() }
                    }
                } catch { if ($listener) { $listener.Stop() } }
            }
            Start-Sleep -Milliseconds 5
        } until ([DateTime]::UtcNow -ge $deadline)
        throw 'Could not inject the proxy listener race'
    }
    $raceDeadline = [DateTime]::UtcNow.AddSeconds(15)
    while (-not (Test-Path -LiteralPath $raceReady) -and [DateTime]::UtcNow -lt $raceDeadline) { Start-Sleep -Milliseconds 100 }
    Confirm-Check (Test-Path -LiteralPath $raceReady) 'Owned port race fixture ready before startup'
    Start-Product $root
    $data = Join-Path $root 'data'
    $state = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('get_runtime_state')"
    Confirm-Check ($state.mode -eq 'Sidecar' -and -not $state.serviceUsable -and -not $state.tunCapable) 'Portable Sidecar without service or TUN'
    $result.runtimeState = $state
    $info = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('get_clash_info')"
    $result.proxyPorts = @{ original = $originalPort; portable = $info.mixed_port }
    Confirm-Check ($info.mixed_port -ne $originalPort -and $info.mixed_port -gt 0) 'Occupied original proxy port avoided'
    Confirm-Check (Test-Path -LiteralPath $raceBound) 'Proxy port occupied after runtime configuration was written'
    $race = Get-Content -LiteralPath $raceBound -Encoding UTF8 -Raw | ConvertFrom-Json
    $result.portRace = $race
    Confirm-Check ($info.mixed_port -ne $race.port -and $portRaceJob.State -eq 'Running' -and (Get-Process -Id $originalCoreId -ErrorAction SilentlyContinue)) 'Port race retries on another listener without terminating foreign processes'
    Stop-Job $portRaceJob
    Remove-Job $portRaceJob
    $portRaceJob = $null
    $document = Invoke-DebuggerExpression "({title:document.title,text:document.body.innerText.slice(0,2400),rootChildren:document.getElementById('root')?.childElementCount ?? 0})"
    Confirm-Check ($document.title -eq 'ChainDesk' -and $document.rootChildren -gt 0 -and $document.text -match 'ChainDesk') 'ChainDesk interface rendered in Unicode path with spaces'
    $result.document = $document
    foreach ($directory in @('logs', 'backups', 'runtime', 'temp', 'webview')) {
        Confirm-Check (Test-Path -LiteralPath (Join-Path $data $directory)) "Portable $directory directory exists"
    }
    Confirm-Check (-not (Test-Path -LiteralPath (Join-Path $env:APPDATA 'local.chaindesk.portable'))) 'No product AppData fallback'
    Confirm-Check (-not (Test-Path 'C:\M0\forbidden-inherited-browser-data') -and -not (Test-Path 'C:\M0\forbidden-inherited-core-data')) 'Inherited runtime and core paths ignored'
    $core = @(Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $application.Id -and $_.Name -eq 'mihomo.exe' })
    Confirm-Check ($core.Count -eq 1 -and $core[0].ExecutablePath -eq (Join-Path $root 'core\mihomo.exe') -and $core[0].CommandLine -notmatch 'ext-ctl-pipe') 'Only product core launched from explicit path without native pipe'
    $coreId = $core[0].ProcessId
    $runtimeFile = Get-ChildItem -LiteralPath (Join-Path $data 'runtime') -Filter 'clash-verge.yaml' -Recurse -File | Select-Object -First 1
    Confirm-Check ($null -ne $runtimeFile) 'Session configuration stored inside portable data'
    $acl = Get-Acl -LiteralPath $runtimeFile.DirectoryName
    $allowedSids = @([Security.Principal.WindowsIdentity]::GetCurrent().User.Value, 'S-1-5-18')
    $unexpectedAces = @($acl.Access | Where-Object { $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value -notin $allowedSids })
    Confirm-Check ($acl.AreAccessRulesProtected -and $unexpectedAces.Count -eq 0 -and $acl.Access.Count -eq 2) 'Session DACL grants only current user and SYSTEM'
    $yaml = [IO.File]::ReadAllText($runtimeFile.FullName)
    $secret = [regex]::Match($yaml, '(?m)^secret:\s*["'']?([0-9a-f]{64})').Groups[1].Value
    $controllerPort = [regex]::Match($yaml, '(?m)^external-controller:\s*["'']?127\.0\.0\.1:(\d+)').Groups[1].Value
    Confirm-Check ($secret.Length -eq 64 -and [int]$controllerPort -gt 0 -and $yaml -notmatch 'external-controller-pipe') 'Random authenticated loopback controller without pipe'
    $unauthorized = 0
    try { Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$controllerPort/version" -TimeoutSec 5 | Out-Null } catch { $unauthorized = [int]$_.Exception.Response.StatusCode }
    Confirm-Check ($unauthorized -eq 401) 'Unauthenticated core control rejected'
    $version = Invoke-RestMethod -Uri "http://127.0.0.1:$controllerPort/version" -Headers @{ Authorization = "Bearer $secret" } -TimeoutSec 5
    Confirm-Check ($version.version -eq 'v1.19.33') 'Pinned core version authenticated'
    $baseConfig = Invoke-RestMethod -Uri "http://127.0.0.1:$controllerPort/configs" -Headers @{ Authorization = "Bearer $secret" } -TimeoutSec 5
    Confirm-Check (-not $baseConfig.'allow-lan' -and -not $baseConfig.tun.enable -and -not $baseConfig.ipv6 -and $baseConfig.'bind-address' -eq '127.0.0.1') 'Live core enforces loopback, disabled TUN and disabled IPv6'
    $visibleYaml = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('get_runtime_yaml')"
    Confirm-Check ([regex]::Match($visibleYaml, '(?m)^mixed-port:\s*(\d+)').Groups[1].Value -eq [string]$info.mixed_port) 'Frontend runtime preview reflects the recovered proxy port'
    Confirm-Check ($visibleYaml -notmatch [regex]::Escape($secret) -and -not $info.secret) 'Controller token excluded from frontend configuration'
    $visibleBase = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('plugin:mihomo|get_base_config')"
    Confirm-Check (($visibleBase | ConvertTo-Json -Depth 10 -Compress) -notmatch [regex]::Escape($secret)) 'Permitted core configuration API does not expose controller token'
    foreach ($command in @('install_service', 'upgrade_clash_core', 'plugin:mihomo|update_secret', 'plugin:mihomo|patch_base_config')) {
        $literal = $command | ConvertTo-Json -Compress
        $rejected = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke($literal, {}).then(() => false, () => true)"
        Confirm-Check ($rejected -eq $true) "Unsupported command rejected: $command"
    }
    $rejected = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('patch_verge_config', {payload:{enable_tun_mode:true}}).then(() => false, () => true)"
    Confirm-Check ($rejected -eq $true) 'TUN enable rejected by backend'
    $probe = Invoke-WebRequest -UseBasicParsing -Uri "$Controller/probe" -Proxy "http://127.0.0.1:$($info.mixed_port)" -TimeoutSec 15
    Confirm-Check ($probe.Content -eq 'chaindesk-m0-proxy-ok') 'Product proxy works while original core runs'
    $second = Start-Process -FilePath (Join-Path $root 'ChainDesk.exe') -PassThru
    Confirm-Check ($second.WaitForExit(30000) -and $second.ExitCode -eq 0 -and -not $application.HasExited) 'Duplicate product launch activates existing instance'
    Confirm-Check ((Get-OriginalFingerprint) -eq $originalFingerprint -and (Get-ProxySnapshot) -eq $initialProxy) 'Portable startup preserves original data and system proxy'
    Set-Proxy $originalSocket $true
    $foreignProxy = Get-ProxySnapshot
    $originalFingerprint = Get-OriginalFingerprint
    $conflict = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('patch_verge_config', {payload:{enable_system_proxy:true}}).then(() => ({rejected:false}), e => ({rejected:true,message:JSON.stringify(e)}))"
    Confirm-Check ($conflict.rejected -and $conflict.message -match 'Another application owns') 'Foreign proxy ownership produces actionable conflict'
    Confirm-Check ((Get-ProxySnapshot) -eq $foreignProxy -and (Get-OriginalFingerprint) -eq $originalFingerprint) 'Rejected proxy takeover preserves foreign settings and data'
    Set-Proxy $originalSocket $false
    [void](Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('patch_verge_config', {payload:{proxy_auto_config:true}})" $originalSocket)
    Set-Proxy $originalSocket $true
    $foreignPac = Get-ProxySnapshot
    $conflict = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('patch_verge_config', {payload:{enable_system_proxy:true}}).then(() => false, () => true)"
    Confirm-Check ($conflict -eq $true -and (Get-ProxySnapshot) -eq $foreignPac) 'Foreign active PAC is preserved even with global proxy disabled'
    Set-Proxy $originalSocket $false
    [void](Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('patch_verge_config', {payload:{proxy_auto_config:false}})" $originalSocket)
    $beforeAcquisition = Get-ProxySnapshot
    Set-Proxy $socket $true
    $activeProxy = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings'
    Confirm-Check ($activeProxy.ProxyEnable -eq 1 -and $activeProxy.ProxyServer -match "127\.0\.0\.1:$($info.mixed_port)") 'Product acquires inactive system proxy'
    Set-Proxy $socket $false
    Confirm-Check ((Get-ProxySnapshot) -eq $beforeAcquisition) 'Product disconnect restores exact original proxy values'
    [void](Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('patch_verge_config', {payload:{proxy_auto_config:true}})")
    Set-Proxy $socket $true
    $activePac = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings'
    Confirm-Check ($activePac.ProxyEnable -eq 0 -and $activePac.AutoConfigURL -match '^http://127\.0\.0\.1:\d+/commands/pac$') 'Product acquires PAC on its own loopback server'
    Set-Proxy $socket $false
    Confirm-Check ((Get-ProxySnapshot) -eq $beforeAcquisition) 'PAC disconnect restores exact original proxy values'
    [void](Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('patch_verge_config', {payload:{proxy_auto_config:false}})")
    Set-Proxy $socket $true
    Set-Proxy $originalSocket $true
    $foreignProxy = Get-ProxySnapshot
    Stop-Application $application $socket
    $socket = $null
    Confirm-Check ((Get-ProxySnapshot) -eq $foreignProxy) 'Normal exit preserves proxy changed by original client'
    Confirm-Check (-not (Get-Process -Id $coreId -ErrorAction SilentlyContinue) -and (Get-Process -Id $originalCoreId -ErrorAction SilentlyContinue)) 'Normal exit stops only product core'
    Set-Proxy $originalSocket $false
    # The upstream client intentionally retains inactive endpoint values; establish a new boundary after that fixture action.
    $beforeAcquisition = Get-ProxySnapshot
    $originalFingerprint = Get-OriginalFingerprint
    Start-Product $root
    $core = @(Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $application.Id -and $_.Name -eq 'mihomo.exe' })
    $coreId = $core[0].ProcessId
    Set-Proxy $socket $false
    $application.Kill()
    $application.WaitForExit()
    $socket.Dispose()
    $socket = $null
    Start-Sleep -Seconds 2
    Confirm-Check (-not (Get-Process -Id $coreId -ErrorAction SilentlyContinue) -and (Get-Process -Id $originalCoreId -ErrorAction SilentlyContinue)) 'Forced parent termination closes owned core job only'
    $moved = Join-Path $caseRoot 'moved portable'
    Move-Item -LiteralPath $root -Destination $moved
    $root = $moved
    $data = Join-Path $root 'data'
    Start-Product $root
    Confirm-Check ((Test-Path -LiteralPath (Join-Path $root 'data\webview')) -and -not (Test-Path -LiteralPath (Join-Path $workspace 'ChainDesk'))) 'Portable folder restarts after move'
    Set-Proxy $socket $true
    Stop-Application $application $socket
    $socket = $null
    Confirm-Check ((Get-ProxySnapshot) -eq $beforeAcquisition) 'Normal exit restores product-owned system proxy'
    Confirm-Check ((Get-OriginalFingerprint) -eq $originalFingerprint -and (Get-Process -Id $originalCoreId -ErrorAction SilentlyContinue)) 'Crash, move and restart preserve original client'
    Confirm-Check ($protocolsBefore -eq (Get-ProtocolFingerprint)) 'Upstream URL associations and commands unchanged'
    Confirm-Check (@(Get-Service | Where-Object { $_.Name -match 'clash|mihomo|verge' }).Count -eq $servicesBefore) 'No proxy service installed during acceptance'
    $readOnly = Join-Path $caseRoot 'read only'
    New-Item -ItemType Directory -Path $readOnly | Out-Null
    Copy-Item -LiteralPath (Join-Path $root 'ChainDesk.exe') -Destination $readOnly
    $restrictedAcl = Get-Acl -LiteralPath $readOnly
    $restrictedAcl.SetAccessRuleProtection($true, $false)
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $restrictedAcl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($sid, 'ReadAndExecute', 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
    Set-Acl -LiteralPath $readOnly -AclObject $restrictedAcl
    Confirm-StartupRefusal (Join-Path $readOnly 'ChainDesk.exe') 'not writable'
    Confirm-Check (-not (Test-Path -LiteralPath (Join-Path $readOnly 'data')) -and -not (Test-Path -LiteralPath (Join-Path $env:APPDATA 'local.chaindesk.portable'))) 'Unwritable location creates no fallback data'
    Start-Product $root
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing
    $screen = [Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bitmap = [Drawing.Bitmap]::new($screen.Width, $screen.Height)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $image = [IO.MemoryStream]::new()
    try {
        $graphics.CopyFromScreen($screen.Location, [Drawing.Point]::Empty, $screen.Size)
        $bitmap.Save($image, [Drawing.Imaging.ImageFormat]::Png)
        $image.Position = 0
        $result.screenshotSha256 = (Get-FileHash -InputStream $image -Algorithm SHA256).Hash.ToLowerInvariant()
        Invoke-WebRequest -UseBasicParsing -Method Post -Uri "$Controller/screenshot" -ContentType 'image/png' -Body $image.ToArray() -TimeoutSec 30 | Out-Null
    } finally { $image.Dispose(); $graphics.Dispose(); $bitmap.Dispose() }
    Stop-Application $application $socket
    $socket = $null
    Stop-Application $original $originalSocket
    $originalSocket = $null
    $result.passed = $true
} catch {
    $result.error = $_.Exception.Message
    $result.scriptStack = $_.ScriptStackTrace
    if ($originalData) {
        $result.originalLogTails = @(Get-ChildItem -LiteralPath (Join-Path $originalData 'logs') -File -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 2 | ForEach-Object {
            @{ name = $_.Name; lines = @(Get-Content -LiteralPath $_.FullName -Encoding UTF8 -Tail 65 -ErrorAction SilentlyContinue | ForEach-Object { [string]$_ }) }
        })
    }
    if ($data) {
        $result.logTails = @(Get-ChildItem -LiteralPath (Join-Path $data 'logs') -File -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 2 | ForEach-Object {
            @{ name = $_.Name; lines = @(Get-Content -LiteralPath $_.FullName -Encoding UTF8 -Tail 45 -ErrorAction SilentlyContinue | ForEach-Object { [string]$_ }) }
        })
    }
} finally {
    if ($portRaceJob) { Stop-Job $portRaceJob; Remove-Job $portRaceJob }
    foreach ($debugger in @($socket, $originalSocket)) { if ($debugger) { $debugger.Dispose() } }
    foreach ($process in @($application, $original)) { if ($process -and -not $process.HasExited) { $process.Kill(); $process.WaitForExit() } }
    $result.finishedAt = [DateTime]::UtcNow.ToString('o')
    $json = $result | ConvertTo-Json -Depth 10
    [IO.File]::WriteAllText((Join-Path $PSScriptRoot 'result.json'), $json, [Text.UTF8Encoding]::new($false))
    Invoke-WebRequest -UseBasicParsing -Method Post -Uri "$Controller/result" -ContentType 'application/json; charset=utf-8' -Body ([Text.Encoding]::UTF8.GetBytes($json)) -TimeoutSec 30 | Out-Null
    & shutdown.exe /s /t 0
}
if (-not $result.passed) { exit 1 }
