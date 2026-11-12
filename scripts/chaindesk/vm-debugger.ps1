function Invoke-DebuggerExpression([string]$Expression, [Net.WebSockets.ClientWebSocket]$DebuggerSocket = $script:socket) {
    $script:requestId++
    $request = @{ id = $script:requestId; method = 'Runtime.evaluate'; params = @{ expression = $Expression; awaitPromise = $true; returnByValue = $true } }
    $bytes = [Text.Encoding]::UTF8.GetBytes(($request | ConvertTo-Json -Depth 8 -Compress))
    $timeout = [Threading.CancellationTokenSource]::new(15000)
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
