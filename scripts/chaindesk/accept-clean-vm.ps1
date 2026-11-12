param([Parameter(Mandatory = $true)][string]$Controller)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$machine = Get-CimInstance Win32_ComputerSystem
if ($env:COMPUTERNAME -ne 'CHAINDESK-M0' -or $machine.Manufacturer -notmatch 'QEMU' -or $PSScriptRoot -ne 'C:\M0') {
    throw 'This acceptance script may run only in the disposable CHAINDESK-M0 QEMU guest'
}
$endpoint = [Uri]$Controller
if ($endpoint.Scheme -ne 'http' -or $endpoint.Host -ne '10.0.2.2' -or $endpoint.Port -lt 1024) {
    throw 'The controller must be the QEMU host-loopback alias'
}

$result = [ordered]@{ passed = $false; machine = $machine.Model; manufacturer = $machine.Manufacturer }
$result.harnessSha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
$application = $null
$socket = $null
$requestId = 0

. (Join-Path $PSScriptRoot 'vm-debugger.ps1')

try {
    $identity = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
    if ($identity.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Acceptance requires a standard user token' }
    $result.standardUser = $true
    $windows = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
    $result.windows = @{ build = [int]$windows.CurrentBuildNumber; revision = $windows.UBR; version = $windows.DisplayVersion; architecture = $env:PROCESSOR_ARCHITECTURE }
    if ($result.windows.build -lt 26200 -or $result.windows.architecture -ne 'AMD64') { throw 'Windows 11 build 26200+ x64 is required' }
    $developerTools = @('node', 'pnpm', 'cargo', 'rustc', 'cl', 'git' | Where-Object { Get-Command $_ -ErrorAction SilentlyContinue })
    if ($developerTools.Count) { throw "Development tools found: $developerTools" }
    $result.developerToolsAbsent = $true
    $servicesBefore = @(Get-Service | Where-Object { $_.Name -match 'clash|mihomo|verge' })
    if ($servicesBefore.Count) { throw 'An existing proxy service was found' }
    $appData = Join-Path $env:APPDATA 'io.github.clash-verge-rev.clash-verge-rev'
    if (Test-Path -LiteralPath $appData) { throw 'Existing upstream application data was found' }
    $result.initialApplicationDataAbsent = $true
    $result.servicesBefore = $servicesBefore.Count
    $proxyBefore = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings' | Select-Object ProxyEnable, ProxyServer, AutoConfigURL | ConvertTo-Json -Compress
    New-Item -ItemType Directory -Path $appData | Out-Null
    [IO.File]::WriteAllText((Join-Path $appData 'verge.yaml'), "auto_check_update: false`n", [Text.UTF8Encoding]::new($false))
    $result.updaterDisabledByFixture = $true

    $manifest = Invoke-RestMethod -Uri "$Controller/manifest.json" -TimeoutSec 30
    $archive = Join-Path $env:TEMP 'chaindesk-m0-payload.zip'
    & curl.exe --fail --silent --show-error --max-time 300 --output $archive "$Controller/payload.zip"
    if ($LASTEXITCODE -ne 0) { throw 'Acceptance payload download failed' }
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $manifest.archiveSha256) { throw 'Acceptance payload checksum mismatch' }
    $appDirectory = 'C:\M0\app'
    New-Item -ItemType Directory -Path $appDirectory | Out-Null
    & tar.exe -xf $archive -C $appDirectory
    if ($LASTEXITCODE -ne 0) { throw 'Acceptance payload extraction failed' }
    foreach ($file in $manifest.files) {
        $target = [IO.Path]::GetFullPath((Join-Path $appDirectory $file.path))
        if (-not $target.StartsWith($appDirectory + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Payload path escapes the application directory' }
        if ((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash.ToLowerInvariant() -ne $file.sha256) { throw "Payload file checksum mismatch: $($file.path)" }
    }
    $result.payloadSha256 = $manifest.archiveSha256
    $result.sourceCommit = $manifest.sourceCommit
    $result.files = $manifest.files
    $runtime = Join-Path $appDirectory $manifest.runtimeDirectory
    $env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER = $runtime
    $env:WEBVIEW2_USER_DATA_FOLDER = 'C:\M0\browser-data'
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
    $application = Start-Process -FilePath (Join-Path $appDirectory 'clash-verge.exe') -WorkingDirectory $appDirectory -PassThru
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    do {
        Start-Sleep -Milliseconds 500
        if ($application.HasExited) { throw "Application exited during startup: $($application.ExitCode)" }
        try { $pages = Invoke-RestMethod -Uri 'http://127.0.0.1:9222/json/list' -TimeoutSec 2 } catch { $pages = @() }
        $page = $pages | Where-Object { $_.type -eq 'page' -and $_.webSocketDebuggerUrl } | Select-Object -First 1
    } until ($page -or [DateTime]::UtcNow -ge $deadline)
    if (-not $page) { throw 'The application WebView did not become available' }
    $socket = [Net.WebSockets.ClientWebSocket]::new()
    $timeout = [Threading.CancellationTokenSource]::new(10000)
    try { [void]$socket.ConnectAsync([Uri]$page.webSocketDebuggerUrl, $timeout.Token).GetAwaiter().GetResult() } finally { $timeout.Dispose() }
    do {
        $ready = Invoke-DebuggerExpression "typeof window.__TAURI_INTERNALS__?.invoke === 'function'"
        if ($ready) {
            $state = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('get_runtime_state')"
            if ($state.mode -eq 'Sidecar') { break }
        }
        Start-Sleep -Milliseconds 500
    } until ([DateTime]::UtcNow -ge $deadline)
    if ($state.mode -ne 'Sidecar' -or $state.isAdmin -or $state.serviceUsable -or $state.tunCapable) { throw "Unexpected runtime state: $($state | ConvertTo-Json -Compress)" }
    $result.runtimeState = $state
    $mixedPort = Invoke-DebuggerExpression "window.__TAURI_INTERNALS__.invoke('get_clash_info').then(x => x.mixed_port)"
    if ($mixedPort -lt 1 -or $mixedPort -gt 65535) { throw 'The application returned an invalid proxy port' }
    $probe = Invoke-WebRequest -UseBasicParsing -Uri "$Controller/probe" -Proxy "http://127.0.0.1:$mixedPort" -TimeoutSec 15
    if ($probe.Content -ne 'chaindesk-m0-proxy-ok') { throw 'The application-owned proxy failed its isolated HTTP probe' }
    $result.proxyRequestPassed = $true
    $document = Invoke-DebuggerExpression "({title:document.title,text:document.body.innerText.slice(0,2000),rootChildren:document.getElementById('root')?.childElementCount ?? 0})"
    if ($document.rootChildren -lt 1 -or $document.text.Length -lt 30) { throw 'The application interface did not render' }
    $result.document = $document
    $application.Refresh()
    if ($application.MainWindowHandle -eq [IntPtr]::Zero) { throw 'No application window is visible' }
    $core = @(Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $application.Id -and $_.Name -eq 'verge-mihomo.exe' })
    if ($core.Count -ne 1) { throw 'Expected exactly one application-owned Sidecar core' }
    $coreProcessId = $core[0].ProcessId
    $result.coreProcessId = $coreProcessId
    $webviews = @(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($runtime + '\', [StringComparison]::OrdinalIgnoreCase) })
    if (-not $webviews.Count) { throw 'The fixed WebView runtime was not used' }
    $result.fixedWebviewUsed = $true
    $result.applicationDataCreated = (Test-Path -LiteralPath (Join-Path $appData 'config.yaml')) -and (Test-Path -LiteralPath (Join-Path $appData 'profiles.yaml'))
    if (-not $result.applicationDataCreated) { throw 'The application data directory was not initialized' }
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
    [void](Invoke-DebuggerExpression "setTimeout(() => window.__TAURI_INTERNALS__.invoke('exit_app'), 100); true")
    if (-not $application.WaitForExit(30000)) { throw 'The application did not exit through its normal IPC command' }
    if (Get-Process -Id $coreProcessId -ErrorAction SilentlyContinue) { throw 'The Sidecar core survived normal application exit' }
    $result.normalExitCode = $application.ExitCode
    if ($application.ExitCode -ne 0) { throw 'Application cleanup reported a failure' }
    $result.ownedCoreStopped = $true
    $proxyAfter = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings' | Select-Object ProxyEnable, ProxyServer, AutoConfigURL | ConvertTo-Json -Compress
    if ($proxyBefore -ne $proxyAfter) { throw 'The application changed guest proxy settings unexpectedly' }
    $result.proxySettingsUnchanged = $true
    $result.servicesAfter = @(Get-Service | Where-Object { $_.Name -match 'clash|mihomo|verge' }).Count
    if ($result.servicesAfter -ne 0) { throw 'Application startup installed a proxy service' }
    $result.passed = $true
} catch {
    $result.error = $_.Exception.Message
    $result.scriptStack = $_.ScriptStackTrace
    if ($appData) {
        $result.logTails = @(Get-ChildItem -LiteralPath (Join-Path $appData 'logs') -File -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 2 | ForEach-Object {
            @{ name = $_.Name; lines = @(Get-Content -LiteralPath $_.FullName -Encoding UTF8 -Tail 60 -ErrorAction SilentlyContinue | ForEach-Object { [string]$_ }) }
        })
    }
} finally {
    if ($socket) { $socket.Dispose() }
    if ($application -and -not $application.HasExited) { $application.Kill(); $application.WaitForExit() }
    $result.finishedAt = [DateTime]::UtcNow.ToString('o')
    $json = $result | ConvertTo-Json -Depth 10
    [IO.File]::WriteAllText('C:\M0\result.json', $json, [Text.UTF8Encoding]::new($false))
    Invoke-WebRequest -UseBasicParsing -Method Post -Uri "$Controller/result" -ContentType 'application/json; charset=utf-8' -Body ([Text.Encoding]::UTF8.GetBytes($json)) -TimeoutSec 30 | Out-Null
    if ($result.passed) { & shutdown.exe /s /t 0 }
}
if (-not $result.passed) { exit 1 }
