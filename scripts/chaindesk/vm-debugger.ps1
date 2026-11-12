function Invoke-DebuggerExpression([string]$Expression, [Net.WebSockets.ClientWebSocket]$DebuggerSocket = $script:socket, [int]$TimeoutMilliseconds = 15000) {
    $script:requestId++
    $request = @{ id = $script:requestId; method = 'Runtime.evaluate'; params = @{ expression = $Expression; awaitPromise = $true; returnByValue = $true } }
    $bytes = [Text.Encoding]::UTF8.GetBytes(($request | ConvertTo-Json -Depth 8 -Compress))
    $timeout = [Threading.CancellationTokenSource]::new($TimeoutMilliseconds)
    try {
        [void]$DebuggerSocket.SendAsync([ArraySegment[byte]]::new($bytes), [Net.WebSockets.WebSocketMessageType]::Text, $true, $timeout.Token).GetAwaiter().GetResult()
        do {
            $message = [IO.MemoryStream]::new()
            try {
                do {
                    $buffer = [byte[]]::new(65536)
                    $received = $DebuggerSocket.ReceiveAsync([ArraySegment[byte]]::new($buffer), $timeout.Token).GetAwaiter().GetResult()
                    if ($received.MessageType -eq [Net.WebSockets.WebSocketMessageType]::Close) { throw 'WebView debugger closed unexpectedly' }
                    $message.Write($buffer, 0, $received.Count)
                } until ($received.EndOfMessage)
                $response = [Text.Encoding]::UTF8.GetString($message.ToArray()) | ConvertFrom-Json
            } finally { $message.Dispose() }
        } until ($response.id -eq $script:requestId)
        if ($response.error -or $response.result.exceptionDetails) { throw ($response | ConvertTo-Json -Depth 6 -Compress) }
        return $response.result.result.value
    } finally { $timeout.Dispose() }
}

function Confirm-VmFixtureCertificate([int]$FixtureProcessId, [string]$ExpectedThumbprint) {
    if ($env:COMPUTERNAME -ne 'CHAINDESK-M0' -or (Get-CimInstance Win32_ComputerSystem).Manufacturer -notmatch 'QEMU' -or $ExpectedThumbprint -notmatch '^[A-F0-9]{40}$') { throw 'Disposable guest fixture certificate required' }
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.RegularExpressions;
public static class FixtureCertificateNative {
    private delegate bool EnumerateWindow(IntPtr window, IntPtr argument);
    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumerateWindow callback, IntPtr argument);
    [DllImport("user32.dll")]
    private static extern bool EnumChildWindows(IntPtr parent, EnumerateWindow callback, IntPtr argument);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetWindowTextW(IntPtr window, StringBuilder text, int capacity);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool PostMessageW(IntPtr window, uint message, UIntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll")]
    private static extern int GetDlgCtrlID(IntPtr control);
    private static string Text(IntPtr window) {
        var text = new StringBuilder(8192);
        GetWindowTextW(window, text, text.Capacity);
        return text.ToString();
    }
    public static bool Approve(uint owner, string thumbprint) {
        IntPtr dialog = IntPtr.Zero, yes = IntPtr.Zero;
        EnumWindows((window, ignored) => {
            uint actual;
            GetWindowThreadProcessId(window, out actual);
            if (actual != owner || Text(window) != "Security Warning") return true;
            var text = new StringBuilder();
            IntPtr candidate = IntPtr.Zero;
            EnumChildWindows(window, (child, unused) => {
                var label = Text(child);
                text.Append(label);
                if (label.Replace("&", "") == "Yes") candidate = child;
                return true;
            }, IntPtr.Zero);
            if (candidate == IntPtr.Zero || !text.ToString().Contains("ChainDesk_VM_fixture_only") ||
                Regex.Replace(text.ToString(), "[^a-fA-F0-9]", "").IndexOf(thumbprint, StringComparison.OrdinalIgnoreCase) < 0) return true;
            dialog = window;
            yes = candidate;
            return false;
        }, IntPtr.Zero);
        if (dialog == IntPtr.Zero) return false;
        uint confirmed;
        GetWindowThreadProcessId(dialog, out confirmed);
        int button = GetDlgCtrlID(yes);
        return confirmed == owner && button > 0 && PostMessageW(dialog, 273, new UIntPtr((uint)button), yes);
    }
    public static string Describe() {
        var windows = new List<string>();
        EnumWindows((window, ignored) => {
            var title = Text(window);
            if (title == "Security Warning") {
                uint owner;
                GetWindowThreadProcessId(window, out owner);
                var text = new StringBuilder(title + " PID=" + owner + ": ");
                EnumChildWindows(window, (child, unused) => { text.Append(Text(child)).Append(" "); return true; }, IntPtr.Zero);
                windows.Add(text.ToString());
            }
            return true;
        }, IntPtr.Zero);
        return String.Join("; ", windows);
    }
}
'@
    if ($FixtureProcessId -eq 0) {
        $owners = @(Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'clash-verge.exe' -and $_.ExecutablePath -like 'C:\M0\M1\case-*\original app\clash-verge.exe' })
        if ($owners.Count -ne 1) { throw 'Exactly one owned original fixture is required' }
        $FixtureProcessId = $owners[0].ParentProcessId
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    do {
        if (-not (Get-Process -Id $FixtureProcessId -ErrorAction SilentlyContinue)) { return }
        if ([FixtureCertificateNative]::Approve([uint32]$FixtureProcessId, $ExpectedThumbprint)) { return }
        Start-Sleep -Milliseconds 200
    } until ([DateTime]::UtcNow -ge $deadline)
    throw ('The disposable guest fixture CA prompt could not be verified: ' + [FixtureCertificateNative]::Describe())
}

function Send-VmEvidence([string]$Uri, [byte[]]$Body, [string]$ContentType = 'application/json; charset=utf-8') {
    if ($env:COMPUTERNAME -ne 'CHAINDESK-M0' -or ([Uri]$Uri).Host -ne '10.0.2.2') { throw 'Disposable guest controller required' }
    $temporary = Join-Path 'C:\M0\M1' ('evidence-' + [Guid]::NewGuid().ToString('N') + '.bin')
    try {
        [IO.File]::WriteAllBytes($temporary, $Body)
        & (Join-Path $env:SystemRoot 'System32\curl.exe') --noproxy 10.0.2.2 --fail --silent --show-error --connect-timeout 3 --max-time 15 --header "Content-Type: $ContentType" --data-binary "@$temporary" $Uri | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "Guest evidence upload failed: $LASTEXITCODE" }
    } finally { Remove-Item -LiteralPath $temporary -ErrorAction SilentlyContinue }
}

function Send-VmDiagnostic([string]$Controller) {
    if ($env:COMPUTERNAME -ne 'CHAINDESK-M0' -or -not (Test-Path -LiteralPath 'C:\M0\M1')) { throw 'Disposable guest required' }
    if (-not $Controller) {
        $history = Get-ItemProperty -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\RunMRU'
        $latest = $history.($history.MRUList.Substring(0, 1))
        $Controller = [regex]::Match($latest, 'http://10\.0\.2\.2:\d+/[a-f0-9]{48}').Value
        if (-not $Controller) { throw 'No current guest controller in the explicit diagnostic command' }
    }
    $windows = @(Get-Process | Where-Object { $_.MainWindowTitle } | Select-Object ProcessName,Id,MainWindowTitle)
    $cases = @(Get-ChildItem -LiteralPath 'C:\M0\M1' -Directory -Filter 'case-*')
    $fixtures = @(foreach ($case in $cases) { foreach ($name in @('front-fixture', 'exit-fixture')) {
        $file = Join-Path $case.FullName "$name\stdout.log"
        if (Test-Path -LiteralPath $file) { @{ fixture = $name; lines = @(Get-Content -LiteralPath $file -Encoding UTF8 -Tail 12 | ForEach-Object { [string]$_ }) } }
    } })
    $processes = @(Get-Process | Where-Object { $_.Path -like 'C:\M0\*' } | Select-Object Name,Id,MainWindowTitle)
    $settings = @(foreach ($case in $cases) {
        $path = Join-Path $case.FullName ('portable ' + [char]0x4fbf + [char]0x643a + '\ChainDesk\data\settings.json')
        if (Test-Path -LiteralPath $path) { Get-Content -LiteralPath $path -Encoding UTF8 -Raw | ConvertFrom-Json | Select-Object selectedFrontId,rememberAuthentication }
    })
    [Net.WebRequest]::DefaultWebProxy = $null
    $savedResult = if (Test-Path -LiteralPath 'C:\M0\M1\result.json') { Get-Content -LiteralPath 'C:\M0\M1\result.json' -Encoding UTF8 -Raw | ConvertFrom-Json } else { $null }
    $report = @{ phase = 'Disposable guest inspection'; windows = $windows; fixtures = $fixtures; processes = $processes; savedResult = $savedResult; settings = $settings } | ConvertTo-Json -Depth 15
    Send-VmEvidence "$Controller/progress" ([Text.Encoding]::UTF8.GetBytes($report))
}
