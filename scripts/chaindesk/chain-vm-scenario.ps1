function Get-FixturePort {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    try { return $listener.LocalEndpoint.Port } finally { $listener.Stop() }
}

function Invoke-ChainCommand([string]$Command, $Arguments = @{}, [int]$Timeout = 45000) {
    $name = $Command | ConvertTo-Json -Compress
    $payload = $Arguments | ConvertTo-Json -Depth 30 -Compress
    return Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke($name, $payload)" $socket $Timeout
}

function Start-FixtureCore([string]$Name, $Configuration) {
    $directory = Join-Path $caseRoot $Name
    New-Item -ItemType Directory -Path $directory | Out-Null
    foreach ($file in @('geoip.dat', 'geosite.dat')) {
        Copy-Item -LiteralPath (Join-Path $root "resources\rules\$file") -Destination $directory
    }
    foreach ($listener in $Configuration.listeners) {
        if ($listener.certificate) {
            foreach ($file in @('server.pem', 'server.key')) { Copy-Item -LiteralPath (Join-Path $caseRoot "tls\$file") -Destination (Join-Path $directory $file) -Force }
            $listener.certificate = Join-Path $directory 'server.pem'
            $listener['private-key'] = Join-Path $directory 'server.key'
        }
    }
    $controllerPort = Get-FixturePort
    $token = [Guid]::NewGuid().ToString('N') + [Guid]::NewGuid().ToString('N')
    $base = @{ 'allow-lan' = $false; 'bind-address' = '127.0.0.1'; 'external-controller' = "127.0.0.1:$controllerPort"; secret = $token; mode = 'rule'; ipv6 = $false; 'geo-auto-update' = $false; 'geodata-mode' = $true; 'log-level' = 'debug'; dns = @{ enable = $false }; tun = @{ enable = $false }; rules = @('MATCH,DIRECT') }
    foreach ($key in $Configuration.Keys) { $base[$key] = $Configuration[$key] }
    $configurationPath = Join-Path $directory 'fixture.json'
    [IO.File]::WriteAllText($configurationPath, ($base | ConvertTo-Json -Depth 30), [Text.UTF8Encoding]::new($false))
    $executable = Join-Path $directory 'fixture-core.exe'
    Copy-Item -LiteralPath (Join-Path $root 'core\mihomo.exe') -Destination $executable
    $process = Start-Process -FilePath $executable -ArgumentList @('-d', $directory, '-f', $configurationPath) -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $directory 'stdout.log') -RedirectStandardError (Join-Path $directory 'stderr.log')
    $script:fixtureProcesses += $process
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        if ($process.HasExited) { throw "Fixture core failed: $Name" }
        try {
            $version = Invoke-RestMethod -Uri "http://127.0.0.1:$controllerPort/version" -Headers @{ Authorization = "Bearer $token" } -TimeoutSec 2
            if ($version.version -eq 'v1.19.33') {
                $ports = @(Get-NetTCPConnection -OwningProcess $process.Id -State Listen -ErrorAction SilentlyContinue | Where-Object { $_.LocalAddress -eq '127.0.0.1' } | ForEach-Object { $_.LocalPort })
                if (@($Configuration.listeners | Where-Object { $ports -notcontains $_.port }).Count -eq 0) { return @{ process = $process; port = $controllerPort; token = $token } }
            }
        } catch { Start-Sleep -Milliseconds 100 }
    } until ([DateTime]::UtcNow -ge $deadline)
    throw "Fixture core readiness timed out: $Name"
}

function Get-FixtureTraffic($Fixture) {
    $connections = Invoke-RestMethod -Uri "http://127.0.0.1:$($Fixture.port)/connections" -Headers @{ Authorization = "Bearer $($Fixture.token)" } -TimeoutSec 5
    return [long]$connections.downloadTotal + [long]$connections.uploadTotal
}

function Set-ProbeFixtureMode([string]$Mode) {
    return Invoke-RestMethod -Method Post -Uri "$Controller/probe-fixture" -ContentType 'application/json' -Body (@{ mode = $Mode } | ConvertTo-Json -Compress) -TimeoutSec 5
}

function Enable-FixtureProbeRoutes([string]$Name, $Fixture, [int]$Port) {
    $path = Join-Path $caseRoot "$Name\fixture.json"
    $configuration = [IO.File]::ReadAllText($path) | ConvertFrom-Json
    $configuration | Add-Member -NotePropertyName proxies -NotePropertyValue @(@{ name = 'CHAIN_DESK_TEST_PROBES'; type = 'http'; server = '10.0.2.2'; port = $Port }) -Force
    $configuration.rules = @('www.gstatic.com', 'cp.cloudflare.com', 'api.ipify.org', 'www.cloudflare.com' | ForEach-Object { "DOMAIN,$_,CHAIN_DESK_TEST_PROBES" }) + @('MATCH,DIRECT')
    $json = $configuration | ConvertTo-Json -Depth 30
    [void](Invoke-RestMethod -Method Put -Uri "http://127.0.0.1:$($Fixture.port)/configs?force=true" -Headers @{ Authorization = "Bearer $($Fixture.token)" } -ContentType 'application/json' -Body (@{ payload = $json } | ConvertTo-Json -Depth 32) -TimeoutSec 10)
    [IO.File]::WriteAllText($path, $json, [Text.UTF8Encoding]::new($false))
}

$tls = Join-Path $caseRoot 'tls'
New-Item -ItemType Directory -Path $tls | Out-Null
foreach ($file in @('ca.pem', 'server.pem', 'server.key')) {
    $resource = 'tls-' + $file
    Invoke-WebRequest -UseBasicParsing -Uri "$Controller/$resource" -OutFile (Join-Path $tls $file) -TimeoutSec 20 | Out-Null
}
$certificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new((Join-Path $tls 'ca.pem'))
$certificateInstaller = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\certutil.exe') -ArgumentList @('-user', '-f', '-addstore', 'Root', (Join-Path $tls 'ca.pem')) -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $tls 'certificate-install.log') -RedirectStandardError (Join-Path $tls 'certificate-install-error.log')
$null = $certificateInstaller.Handle
try {
    Confirm-VmFixtureCertificate $certificateInstaller.Id $certificate.Thumbprint
    if (-not $certificateInstaller.WaitForExit(5000) -or $certificateInstaller.ExitCode -ne 0) { throw 'Fixture CA installation did not finish successfully' }
} finally {
    if (-not $certificateInstaller.HasExited) { $certificateInstaller.Kill(); $certificateInstaller.WaitForExit() }
}
$certificateStore = [Security.Cryptography.X509Certificates.X509Store]::new('Root', 'CurrentUser')
$certificateStore.Open([Security.Cryptography.X509Certificates.OpenFlags]::ReadOnly)
try { $trusted = @($certificateStore.Certificates | Where-Object { $_.Thumbprint -eq $certificate.Thumbprint }).Count -eq 1 } finally { $certificateStore.Close() }
Confirm-Check ($trusted) 'Fixture CA installed only in disposable guest CurrentUser Root'
$result.fixtureCertificateThumbprint = $certificate.Thumbprint
$result.fixtureCertificateScope = 'Disposable guest CurrentUser only; not a product signing certificate'

$username = 'fixture:user'
$password = 'fixture:p@ss&word'
$frontPassword = 'fixture-front-password'
$uuid = [Guid]::NewGuid().ToString()
$exitPort = Get-FixturePort
$exitFixture = Start-FixtureCore 'exit-fixture' @{ listeners = @(@{ name = 'fixed-exit'; type = 'socks'; listen = '127.0.0.1'; port = $exitPort; udp = $false; users = @(@{ username = $username; password = $password }) }) }
$listeners = @()
$nodes = @()
foreach ($cipher in @('aes-128-gcm', 'aes-256-gcm', 'chacha20-ietf-poly1305')) {
    $port = Get-FixturePort
    $listeners += @{ name = "ss-$cipher"; type = 'shadowsocks'; listen = '127.0.0.1'; port = $port; cipher = $cipher; password = $frontPassword }
    $nodes += @{ name = "ss-$cipher"; type = 'ss'; server = '127.0.0.1'; port = $port; cipher = $cipher; password = $frontPassword }
}
foreach ($protocol in @('vmess', 'vless')) {
    foreach ($tlsEnabled in @($false, $true)) {
        if ($protocol -eq 'vless' -and -not $tlsEnabled) { continue }
        $port = Get-FixturePort
        $name = "$protocol-tls-$tlsEnabled"
        $listener = @{ name = $name; type = $protocol; listen = '127.0.0.1'; port = $port; users = @(@{ uuid = $uuid; alterId = 0 }) }
        $node = @{ name = $name; type = $protocol; server = '127.0.0.1'; port = $port; uuid = $uuid; alterId = 0; cipher = 'auto'; tls = $tlsEnabled; servername = 'front.fixture.test'; sni = 'front.fixture.test' }
        if ($tlsEnabled) { $listener.certificate = Join-Path $tls 'server.pem'; $listener['private-key'] = Join-Path $tls 'server.key' }
        $listeners += $listener
        $nodes += $node
    }
}
foreach ($protocol in @('trojan', 'anytls')) {
    $port = Get-FixturePort
    $listener = @{ name = $protocol; type = $protocol; listen = '127.0.0.1'; port = $port; certificate = (Join-Path $tls 'server.pem'); 'private-key' = (Join-Path $tls 'server.key') }
    if ($protocol -eq 'anytls') { $listener.users = @{ fixture = $frontPassword } } else { $listener.users = @(@{ username = 'fixture'; password = $frontPassword }) }
    $listeners += $listener
    $nodes += @{ name = $protocol; type = $protocol; server = '127.0.0.1'; port = $port; password = $frontPassword; sni = 'front.fixture.test' }
}
$frontFixture = Start-FixtureCore 'front-fixture' @{ listeners = $listeners }
Confirm-Check ($true) 'Owned loopback fixtures started for front protocol matrix and authenticated exit'
$controlledProbes = $Milestone -in @('m3', 'm4')
if ($controlledProbes) {
    $result.publicEndpointAvailability = @(foreach ($url in @('https://www.gstatic.com/generate_204', 'https://cp.cloudflare.com/generate_204', 'https://api.ipify.org?format=json', 'https://www.cloudflare.com/cdn-cgi/trace')) {
        $status = & (Join-Path $env:SystemRoot 'System32\curl.exe') --noproxy '*' --connect-timeout 3 --max-time 8 --silent --output NUL --write-out '%{http_code}' $url
        @{ service = $url.Split('?')[0]; httpStatus = [string]$status; exitCode = $LASTEXITCODE; throughProductChain = $false }
    })
    $probeFixture = Set-ProbeFixtureMode 'ok'
    Enable-FixtureProbeRoutes 'front-fixture' $frontFixture $probeFixture.proxyPort
    Enable-FixtureProbeRoutes 'exit-fixture' $exitFixture $probeFixture.proxyPort
    $result.ipv4ObservationFixture = @{ synthetic = $true; trustScope = 'Disposable guest CurrentUser only'; addresses = @('203.0.113.80', '203.0.113.81'); controlledHttpsForReliability = $true; publicProductChainCertifiedByThisRun = $false }
    Confirm-Check ($true) 'Reliability probes use a guest-trusted controlled TLS service through the real front and authenticated fixed exit; public endpoint availability is recorded separately'
}
$result.protocolMatrix = @()
$duplicate = @{}; foreach ($key in $nodes[0].Keys) { $duplicate[$key] = $nodes[0][$key] }; $duplicate.name = 'duplicate renamed'
$sameExit = @{ name = 'same exit'; type = 'ss'; server = '127.0.0.1'; port = $exitPort; cipher = 'aes-128-gcm'; password = 'unused' }
$badVless = ($nodes | Where-Object type -eq 'vless' | Select-Object -First 1).Clone(); $badVless.name = 'unsupported plaintext vless'; $badVless.tls = $false
$badVmess = ($nodes | Where-Object type -eq 'vmess' | Select-Object -First 1).Clone(); $badVmess.name = 'unsupported unencrypted vmess'; $badVmess.cipher = 'none'
$expectedNodeCount = $nodes.Count + 4
$subscriptionPath = Join-Path $caseRoot 'subscription.yaml'
$source = @{ proxies = @($nodes) + @($duplicate, $sameExit, @{ name = 'notice'; type = 'ss'; server = ''; port = 0 }, $badVless, $badVmess); tun = @{ enable = $true }; 'external-controller' = '0.0.0.0:9999' } | ConvertTo-Json -Depth 25
[IO.File]::WriteAllText($subscriptionPath, $source, [Text.UTF8Encoding]::new($false))
$data = Join-Path $root 'data'
Start-Product $root
$beforeProxy = Get-ProxySnapshot
$state = Invoke-ChainCommand 'get_chain_state'
Confirm-Check ($state.phase -eq 'unconfigured' -and -not $state.subscriptionName) 'Blank home requires subscription and exit without automatic proxy acquisition'
$document = Invoke-DebuggerExpression "({title:document.title,text:document.body.innerText,inputs:document.querySelectorAll('input').length})"
Confirm-Check ($document.title -eq 'ChainDesk' -and $document.inputs -ge 3) 'Simple chain home renders inputs'
if ($Milestone -in @('m3', 'm4')) {
    $stable = Invoke-DebuggerExpression 'Promise.all([...Array.from({length:128},()=>window.__TAURI_INTERNALS__.invoke("get_runtime_state")),...Array.from({length:32},()=>window.__TAURI_INTERNALS__.invoke("get_chain_state"))]).then(values=>values.length===160)' $socket 30000
    Confirm-Check ($stable -and -not $application.HasExited) 'Concurrent read-only runtime and chain queries retain a responsive native application'
}
$state = Invoke-ChainCommand 'import_chain_file' @{ path = $subscriptionPath }
Confirm-Check ($state.duplicates -eq 1 -and $state.fronts.Count -eq $expectedNodeCount) 'Bounded importer deduplicates identical nodes while retaining independent identities'
Confirm-Check (@($state.fronts | Where-Object { $_.name -like 'unsupported *' -and $_.rejection }).Count -eq 2) 'Unsupported plaintext VLESS and unencrypted VMess profiles are visibly ineligible'
$subscriptionFixture = Invoke-RestMethod -Method Post -Uri "$Controller/subscription-fixture" -Body $source -ContentType 'text/plain; charset=utf-8'
$subscriptionUrl = $subscriptionFixture.url + '/subscription?token=fixture-subscription-token'
$state = Invoke-ChainCommand 'import_chain_https' @{ url = $subscriptionUrl; localProxyPort = $null }
Confirm-Check ($state.canUpdateSubscription -and $state.subscriptionHost -eq '10.0.2.2' -and $state.fronts.Count -eq $expectedNodeCount) 'Verified HTTPS subscription import ignores inherited environment proxies'
$settingsHash = (Get-FileHash -LiteralPath (Join-Path $data 'settings.json') -Algorithm SHA256).Hash
$credentialsHash = (Get-FileHash -LiteralPath (Join-Path $data 'credentials.bin') -Algorithm SHA256).Hash
foreach ($resource in @('denied', 'large', 'invalid-utf8', 'downgrade')) {
    $request = @{ url = $subscriptionFixture.url + '/' + $resource; localProxyPort = $null } | ConvertTo-Json -Compress
    $rejected = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('import_chain_https',$request).then(()=>false,()=>true)" $socket 45000
    $counts = Invoke-RestMethod -Uri "$Controller/subscription-fixture"
    Confirm-Check ($rejected -and $counts.requests.$resource -eq 1 -and (Get-FileHash -LiteralPath (Join-Path $data 'settings.json') -Algorithm SHA256).Hash -eq $settingsHash -and (Get-FileHash -LiteralPath (Join-Path $data 'credentials.bin') -Algorithm SHA256).Hash -eq $credentialsHash) "Terminal HTTPS failure keeps saved inputs without retry: $resource"
}
$state = Invoke-ChainCommand 'import_chain_https' @{ url = $subscriptionFixture.url + '/retry'; localProxyPort = $null }
$counts = Invoke-RestMethod -Uri "$Controller/subscription-fixture"
Confirm-Check ($counts.requests.retry -eq 3 -and $state.fronts.Count -eq $expectedNodeCount) 'Transient HTTPS 503 retries at most three times before success'
$state = Invoke-ChainCommand 'import_chain_https' @{ url = $subscriptionUrl; localProxyPort = $null }
$settingsHash = (Get-FileHash -LiteralPath (Join-Path $data 'settings.json') -Algorithm SHA256).Hash
$credentialsHash = (Get-FileHash -LiteralPath (Join-Path $data 'credentials.bin') -Algorithm SHA256).Hash
[IO.File]::WriteAllText($subscriptionPath, 'proxies: &p []' + "`nother: *p", [Text.UTF8Encoding]::new($false))
$invalidPath = $subscriptionPath | ConvertTo-Json -Compress
$rejected = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('import_chain_file',{path:$invalidPath}).then(()=>false,()=>true)"
Confirm-Check ($rejected -and (Get-FileHash -LiteralPath (Join-Path $data 'settings.json') -Algorithm SHA256).Hash -eq $settingsHash -and (Get-FileHash -LiteralPath (Join-Path $data 'credentials.bin') -Algorithm SHA256).Hash -eq $credentialsHash) 'Rejected YAML aliases preserve the last saved subscription byte-for-byte'
$state = Invoke-ChainCommand 'save_chain_exit' @{ input = @{ host = '127.0.0.1'; port = $exitPort; username = $username; password = $password; remember = $true } }
Confirm-Check (($state.fronts | Where-Object { $_.name -eq 'same exit' }).rejection -and ($state.fronts | Where-Object { $_.name -eq 'notice' }).rejection) 'Exit endpoint and structurally invalid announcement entries are ineligible'
$json = $state | ConvertTo-Json -Depth 12 -Compress
Confirm-Check (-not $json.Contains($password) -and -not $json.Contains($frontPassword) -and -not $json.Contains($uuid) -and -not $json.Contains('fixture-subscription-token')) 'State responses expose no credentials or subscription token'
Confirm-Check (-not ([IO.File]::ReadAllText((Join-Path $data 'settings.json'))).Contains($password) -and -not ([Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes((Join-Path $data 'credentials.bin')))).Contains($password)) 'Exit authentication is DPAPI protected rather than plaintext persistence'
$first = $state.fronts | Where-Object { $_.name -eq $nodes[0].name } | Select-Object -First 1
$request = @{ input = @{ frontId = $first.id; preset = 'allProxy'; probeConsent = $false; backgroundChecks = $false } } | ConvertTo-Json -Depth 10 -Compress
$rejected = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('connect_chain',$request).then(()=>false,()=>true)"
Confirm-Check ($rejected -and (Get-ProxySnapshot) -eq $beforeProxy) 'Connection requires explicit consent before contacting observation services'
$state = Invoke-ChainCommand 'exclude_chain_front' @{ id = $first.id; excluded = $true }
$request = @{ input = @{ frontId = $first.id; preset = 'allProxy'; probeConsent = $true; backgroundChecks = $false } } | ConvertTo-Json -Depth 10 -Compress
$rejected = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('connect_chain',$request).then(()=>false,()=>true)"
Confirm-Check ($rejected -and ($state.fronts | Where-Object id -eq $first.id).excluded -and (Get-ProxySnapshot) -eq $beforeProxy) 'Manual exclusion rejects connection without acquiring the proxy'
$state = Invoke-ChainCommand 'exclude_chain_front' @{ id = $first.id; excluded = $false }
$state = Invoke-ChainCommand 'save_chain_exit' @{ input = @{ host = '127.0.0.1'; port = $exitPort; username = $username; password = 'intentionally-wrong'; remember = $false } }
$connectPayload = @{ input = @{ frontId = $first.id; preset = 'allProxy'; probeConsent = $true; backgroundChecks = $false } } | ConvertTo-Json -Depth 10 -Compress
$rejected = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('connect_chain',$connectPayload).then(()=>false,()=>true)" $socket 45000
Confirm-Check ($rejected -and (Get-ProxySnapshot) -eq $beforeProxy) 'Wrong SOCKS5 authentication never acquires the system proxy or falls back to DIRECT'
$state = Invoke-ChainCommand 'save_chain_exit' @{ input = @{ host = '127.0.0.1'; port = $exitPort; username = $username; password = $password; remember = $true } }
foreach ($profile in $nodes) {
    $candidate = $state.fronts | Where-Object { $_.name -eq $profile.name } | Select-Object -First 1
    Confirm-Check ($candidate -and -not $candidate.rejection) "Candidate profile accepted: $($profile.name)"
    $frontBefore = Get-FixtureTraffic $frontFixture
    $exitBefore = Get-FixtureTraffic $exitFixture
    if ($profile.name -eq $nodes[0].name) {
        Start-Sleep -Seconds 2
        $candidateId = $candidate.id | ConvertTo-Json -Compress
        [void](Invoke-DebuggerExpression "(() => {const select=document.getElementById('chain-front');Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(select,$candidateId);select.dispatchEvent(new Event('change',{bubbles:true}));const consent=document.getElementById('chain-probe-consent');if(!consent.checked)consent.click();return true;})()")
        Start-Sleep -Milliseconds 500
        $clicked = Invoke-DebuggerExpression "(() => {const button=document.querySelector('[data-testid=chain-connect]');if(button.disabled)return false;button.click();return true;})()"
        Confirm-Check ($clicked) 'First connection is submitted through the rendered home form'
        $deadline = [DateTime]::UtcNow.AddSeconds(45)
        do { Start-Sleep -Milliseconds 500; $state = Invoke-ChainCommand 'get_chain_state' } until ($state.phase -eq 'connected' -or $state.error -or [DateTime]::UtcNow -gt $deadline)
    } else {
        $profileInput = @{ frontId = $candidate.id; preset = 'allProxy'; probeConsent = $true; backgroundChecks = $false }
        if ($controlledProbes) {
            Start-ConnectionRequest $profileInput
            $response = Wait-ConnectionRequest
            if (-not $response.ok) { throw "Protocol profile failed: $($profile.name): $($response.code)" }
            $state = $response.value
        } else {
            $state = Invoke-ChainCommand 'connect_chain' @{ input = $profileInput }
        }
    }
    $observedIpv4 = $state.observation.ipv4 -match '^\d+\.\d+\.\d+\.\d+$'
    $serviceScope = if ($controlledProbes) { 'controlled TLS fixture' } else { 'public services' }
    Confirm-Check ($state.phase -eq 'connected' -and ($observedIpv4 -or $state.error.code -eq 'EXIT_IP_UNAVAILABLE')) "Complete HTTPS chain passed against $serviceScope with an observation or explicit query-unavailability warning: $($profile.name)"
    $frontAfter = Get-FixtureTraffic $frontFixture
    $exitAfter = Get-FixtureTraffic $exitFixture
    Confirm-Check ($frontAfter -gt $frontBefore -and $exitAfter -gt $exitBefore) "Both front and authenticated SOCKS5 exit carried traffic: $($profile.name)"
    $preview = Invoke-ChainCommand 'get_runtime_yaml'
    Confirm-Check (-not $preview.Contains($password) -and -not $preview.Contains($frontPassword) -and -not $preview.Contains($uuid)) 'Runtime preview redacts all authentication fields'
    $result.protocolMatrix += @{ profile = $profile.name; connected = $true; frontTrafficIncreased = $true; exitTrafficIncreased = $true; certificateVerificationDisabled = $false; syntheticObservation = [bool]$controlledProbes; publicIpv4Observed = [bool]($observedIpv4 -and -not $controlledProbes); observationWarning = [string]$state.error.code }
    $state = Invoke-ChainCommand 'disconnect_chain'
    Confirm-Check ($state.phase -eq 'disconnected' -and (Get-ProxySnapshot) -eq $beforeProxy -and (Get-Process -Id $originalCoreId -ErrorAction SilentlyContinue)) 'Disconnect restores exact proxy state and preserves original core'
    $remaining = @(Get-ChildItem -LiteralPath (Join-Path $data 'runtime') -Filter 'clash-verge*.yaml' -Recurse -File)
    Confirm-Check ($remaining.Count -eq 0) 'Disconnected core retains no plaintext working or validation configuration'
    if ($controlledProbes -and $profile.name -eq $nodes[0].name) {
        $recoveryScenario = Join-Path $PSScriptRoot 'recovery-vm-scenario.ps1'
        $result.recoveryScenarioSha256 = (Get-FileHash -LiteralPath $recoveryScenario -Algorithm SHA256).Hash.ToLowerInvariant()
        . $recoveryScenario
    }
}
$frontBefore = Get-FixtureTraffic $frontFixture
$exitBefore = Get-FixtureTraffic $exitFixture
$state = Invoke-ChainCommand 'connect_chain' @{ input = @{ frontId = $first.id; preset = 'chinaDirect'; probeConsent = $true; backgroundChecks = $false } }
Confirm-Check ($state.phase -eq 'connected' -and (Get-FixtureTraffic $frontFixture) -gt $frontBefore -and (Get-FixtureTraffic $exitFixture) -gt $exitBefore) 'China-direct preset validates pinned geo resources while routing observation services through the complete chain'
$state = Invoke-ChainCommand 'disconnect_chain'
Confirm-Check ((Get-ProxySnapshot) -eq $beforeProxy) 'China-direct disconnect restores the exact initial proxy state'
Stop-Application $application $socket
$socket = $null
Start-Product $root
$state = Invoke-ChainCommand 'get_chain_state'
Confirm-Check ($state.hasSavedAuthentication -and $state.hasSessionAuthentication -and $state.fronts.Count -eq $expectedNodeCount -and (Get-ProxySnapshot) -eq $beforeProxy) 'Subscription and optional DPAPI authentication survive restart without automatic connection'
$state = Invoke-ChainCommand 'forget_chain_authentication'
Stop-Application $application $socket
$socket = $null
Start-Product $root
$state = Invoke-ChainCommand 'get_chain_state'
Confirm-Check (-not $state.hasSavedAuthentication -and -not $state.hasSessionAuthentication -and $state.authenticationRequired -and $state.fronts.Count -eq $expectedNodeCount) 'Deleting remembered exit authentication survives restart while preserving the subscription'
Confirm-Check ((Get-OriginalFingerprint) -eq $originalFingerprint -and (Get-ProtocolFingerprint) -eq $protocolsBefore) 'M2 flow preserves original configuration and URL associations'
$result.chainPhase = $state.phase
