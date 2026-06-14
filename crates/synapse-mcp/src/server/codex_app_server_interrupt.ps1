param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$Endpoint,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$ThreadId,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$TurnId,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$ControlPath,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$EventsPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Get-UnixMs {
    return [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
}

function Add-JsonLine([string]$Path, [object]$Value) {
    $json = $Value
    if ($Value -isnot [string]) {
        $json = $Value | ConvertTo-Json -Compress -Depth 100
    }
    Add-Content -LiteralPath $Path -Value $json -Encoding UTF8
}

function Get-JsonProperty($Object, [string]$Name) {
    if ($null -eq $Object) {
        return $null
    }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }
    return $property.Value
}

function Update-Control([string]$Status, [string]$ErrorText) {
    $current = [ordered]@{}
    if (Test-Path -LiteralPath $ControlPath) {
        try {
            $existing = Get-Content -Raw -LiteralPath $ControlPath -Encoding UTF8 | ConvertFrom-Json
            foreach ($property in $existing.PSObject.Properties) {
                $current[$property.Name] = $property.Value
            }
        } catch {
            $current['previous_control_parse_error'] = $_.Exception.Message
        }
    }
    $current['last_interrupt_status'] = $Status
    $current['last_interrupt_error'] = $ErrorText
    $current['last_interrupt_at_unix_ms'] = Get-UnixMs
    $current['updated_at_unix_ms'] = Get-UnixMs
    $tmp = "$ControlPath.tmp.interrupt.$PID"
    $current | ConvertTo-Json -Depth 100 | Set-Content -LiteralPath $tmp -Encoding UTF8
    Move-Item -LiteralPath $tmp -Destination $ControlPath -Force
}

function Send-WebSocketJson($Socket, [object]$Message) {
    $json = $Message | ConvertTo-Json -Compress -Depth 100
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)
    $segment = [ArraySegment[byte]]::new($bytes)
    [void]$Socket.SendAsync($segment, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, [Threading.CancellationToken]::None).GetAwaiter().GetResult()
}

function Receive-WebSocketText($Socket) {
    $buffer = [byte[]]::new(65536)
    $builder = [System.Text.StringBuilder]::new()
    do {
        $segment = [ArraySegment[byte]]::new($buffer)
        $result = $Socket.ReceiveAsync($segment, [Threading.CancellationToken]::None).GetAwaiter().GetResult()
        if ($result.MessageType -eq [System.Net.WebSockets.WebSocketMessageType]::Close) {
            throw 'codex app-server websocket closed before interrupt response'
        }
        [void]$builder.Append([System.Text.Encoding]::UTF8.GetString($buffer, 0, $result.Count))
    } while (-not $result.EndOfMessage)
    return $builder.ToString()
}

function Receive-Response($Socket, [int]$Id) {
    while ($true) {
        $text = Receive-WebSocketText $Socket
        Add-JsonLine -Path $EventsPath -Value $text
        $message = $text | ConvertFrom-Json
        $messageId = Get-JsonProperty $message 'id'
        if ($null -ne $messageId -and [int]$messageId -eq $Id) {
            $responseError = Get-JsonProperty $message 'error'
            if ($null -ne $responseError) {
                $errorJson = $responseError | ConvertTo-Json -Compress -Depth 100
                throw "codex app-server request id $Id failed: $errorJson"
            }
            return $message
        }
    }
}

$socket = $null
try {
    $socket = [System.Net.WebSockets.ClientWebSocket]::new()
    [void]$socket.ConnectAsync([Uri]$Endpoint, [Threading.CancellationToken]::None).GetAwaiter().GetResult()

    Send-WebSocketJson $socket ([ordered]@{
        id = 1
        method = 'initialize'
        params = [ordered]@{
            clientInfo = [ordered]@{ name = 'synapse-agent-interrupt'; version = '0.1.0' }
            capabilities = [ordered]@{ experimentalApi = $true }
        }
    })
    [void](Receive-Response $socket 1)

    Send-WebSocketJson $socket ([ordered]@{
        id = 2
        method = 'turn/interrupt'
        params = [ordered]@{ threadId = $ThreadId; turnId = $TurnId }
    })
    [void](Receive-Response $socket 2)

    Update-Control -Status 'delivered' -ErrorText $null
    ([ordered]@{
        ok = $true
        endpoint = $Endpoint
        thread_id = $ThreadId
        turn_id = $TurnId
        control_path = $ControlPath
        delivered_at_unix_ms = Get-UnixMs
    } | ConvertTo-Json -Compress -Depth 20)
    exit 0
} catch {
    $errorText = $_.Exception.Message
    Update-Control -Status 'failed' -ErrorText $errorText
    ([ordered]@{
        ok = $false
        endpoint = $Endpoint
        thread_id = $ThreadId
        turn_id = $TurnId
        control_path = $ControlPath
        error = $errorText
        failed_at_unix_ms = Get-UnixMs
    } | ConvertTo-Json -Compress -Depth 20)
    exit 1
} finally {
    if ($null -ne $socket) {
        try { $socket.Dispose() } catch {}
    }
}
