param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$SpawnId,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$PromptPath,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$StdoutPath,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$StderrPath,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$FinalMessagePath,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$ControlPath,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$EventsPath,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$AppServerStdoutPath,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$AppServerStderrPath,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$WorkingDir,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$McpUrl,
    [string]$Model = "",
    [string]$NotifyScriptPath = ""
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$Utf8NoBom = [System.Text.UTF8Encoding]::new($false)

function Write-TextNoBom([string]$Path, [string]$Value) {
    [System.IO.File]::WriteAllText($Path, $Value, $Utf8NoBom)
}

function Append-LineNoBom([string]$Path, [string]$Value) {
    [System.IO.File]::AppendAllText($Path, ($Value + [Environment]::NewLine), $Utf8NoBom)
}

function Get-UnixMs {
    return [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
}

function ConvertTo-TomlStringLiteral([string]$Value) {
    return ($Value | ConvertTo-Json -Compress)
}

function Add-JsonLine([string]$Path, [object]$Value) {
    $json = $Value
    if ($Value -isnot [string]) {
        $json = $Value | ConvertTo-Json -Compress -Depth 100
    }
    Append-LineNoBom -Path $Path -Value $json
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

function Write-Control([hashtable]$Patch) {
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
    $current['schema_version'] = 1
    $current['protocol'] = 'codex_app_server_ws'
    $current['endpoint'] = $script:Endpoint
    $current['control_path'] = $ControlPath
    $current['events_path'] = $EventsPath
    $current['app_server_process_id'] = $script:AppServerPid
    $current['thread_id'] = $script:ThreadId
    $current['turn_id'] = $script:TurnId
    $current['turn_status'] = $script:TurnStatus
    $current['last_error'] = $script:LastErrorText
    foreach ($key in $Patch.Keys) {
        $current[$key] = $Patch[$key]
    }
    $current['updated_at_unix_ms'] = Get-UnixMs
    [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($ControlPath)) | Out-Null
    $tmp = "$ControlPath.tmp.$PID"
    Write-TextNoBom -Path $tmp -Value ($current | ConvertTo-Json -Depth 100)
    Move-Item -LiteralPath $tmp -Destination $ControlPath -Force
}

function Get-FreeTcpPort {
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Parse('127.0.0.1'), 0)
    $listener.Start()
    try {
        return [int]$listener.LocalEndpoint.Port
    } finally {
        $listener.Stop()
    }
}

function Wait-AppServerReady([string]$Url, [int]$TimeoutMs) {
    $deadline = [DateTimeOffset]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        try {
            $response = Invoke-WebRequest -UseBasicParsing -Uri $Url -TimeoutSec 1
            if ($response.StatusCode -ge 200 -and $response.StatusCode -lt 300) {
                return
            }
        } catch {
            Start-Sleep -Milliseconds 100
        }
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    throw "codex app-server did not become ready at $Url within ${TimeoutMs}ms"
}

function Get-ChildProcessIds([int]$ParentPid) {
    $children = @(Get-CimInstance Win32_Process -Filter "ParentProcessId = $ParentPid" -ErrorAction SilentlyContinue)
    foreach ($child in $children) {
        Get-ChildProcessIds -ParentPid ([int]$child.ProcessId)
        [int]$child.ProcessId
    }
}

function Stop-OwnedProcessTree([int]$RootPid) {
    $ids = @(Get-ChildProcessIds -ParentPid $RootPid) + @($RootPid)
    foreach ($id in ($ids | Select-Object -Unique)) {
        try {
            $process = Get-Process -Id $id -ErrorAction Stop
            Stop-Process -Id $process.Id -Force -ErrorAction Stop
        } catch {}
    }
}

function Get-CodexLaunchSpec([object[]]$AppArgs) {
    $command = Get-Command codex.ps1 -ErrorAction SilentlyContinue
    if ($null -eq $command) {
        $command = Get-Command codex.cmd -ErrorAction SilentlyContinue
    }
    if ($null -eq $command) {
        $command = Get-Command codex -ErrorAction Stop
    }
    $path = $command.Path
    if ([string]::IsNullOrWhiteSpace($path)) {
        $path = $command.Source
    }
    if ([string]::IsNullOrWhiteSpace($path)) {
        throw 'codex command resolved without an executable path'
    }
    if ($path.EndsWith('.ps1', [StringComparison]::OrdinalIgnoreCase)) {
        return [pscustomobject]@{
            File = 'powershell.exe'
            Args = @('-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', $path) + $AppArgs
        }
    }
    return [pscustomobject]@{ File = $path; Args = $AppArgs }
}

function Connect-AppServer([string]$Endpoint) {
    $socket = [System.Net.WebSockets.ClientWebSocket]::new()
    [void]$socket.ConnectAsync([Uri]$Endpoint, [Threading.CancellationToken]::None).GetAwaiter().GetResult()
    return $socket
}

function Send-WebSocketJson($Socket, [object]$Message) {
    $json = $Message | ConvertTo-Json -Compress -Depth 20
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)
    Add-JsonLine -Path $EventsPath -Value ([ordered]@{
        direction = 'client'
        phase = 'send_start'
        id = Get-JsonProperty $Message 'id'
        method = Get-JsonProperty $Message 'method'
        bytes = $bytes.Length
        at_unix_ms = Get-UnixMs
    })
    $segment = [ArraySegment[byte]]::new($bytes)
    [void]$Socket.SendAsync($segment, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, [Threading.CancellationToken]::None).GetAwaiter().GetResult()
    Add-JsonLine -Path $EventsPath -Value ([ordered]@{
        direction = 'client'
        phase = 'send_ok'
        id = Get-JsonProperty $Message 'id'
        method = Get-JsonProperty $Message 'method'
        bytes = $bytes.Length
        at_unix_ms = Get-UnixMs
    })
}

function Receive-WebSocketText($Socket) {
    $buffer = [byte[]]::new(65536)
    $builder = [System.Text.StringBuilder]::new()
    do {
        $segment = [ArraySegment[byte]]::new($buffer)
        $result = $Socket.ReceiveAsync($segment, [Threading.CancellationToken]::None).GetAwaiter().GetResult()
        if ($result.MessageType -eq [System.Net.WebSockets.WebSocketMessageType]::Close) {
            throw 'codex app-server websocket closed before the turn completed'
        }
        [void]$builder.Append([System.Text.Encoding]::UTF8.GetString($buffer, 0, $result.Count))
    } while (-not $result.EndOfMessage)
    return $builder.ToString()
}

function Read-Message($Socket) {
    $text = Receive-WebSocketText $Socket
    Add-JsonLine -Path $EventsPath -Value $text
    Add-JsonLine -Path $StdoutPath -Value $text
    return ($text | ConvertFrom-Json)
}

function Update-FromNotification($Message) {
    $method = Get-JsonProperty $Message 'method'
    $params = Get-JsonProperty $Message 'params'
    if ($null -eq $method -or $null -eq $params) {
        return
    }
    if ($method -eq 'item/completed') {
        $item = Get-JsonProperty $params 'item'
        $itemType = Get-JsonProperty $item 'type'
        if ($itemType -eq 'agentMessage') {
            $itemText = [string](Get-JsonProperty $item 'text')
            if (-not [string]::IsNullOrWhiteSpace($itemText)) {
                $script:LastAgentMessageText = $itemText
                $phase = [string](Get-JsonProperty $item 'phase')
                if ($phase -eq 'final_answer') {
                    $script:LastFinalAgentMessageText = $itemText
                }
            }
        }
    }
    $turn = Get-JsonProperty $params 'turn'
    $turnId = Get-JsonProperty $turn 'id'
    if ($method -eq 'turn/started' -and $null -ne $turnId) {
        $script:TurnId = [string]$turnId
        $script:TurnStatus = [string](Get-JsonProperty $turn 'status')
        Write-Control @{}
    }
    if ($method -eq 'turn/completed' -and $null -ne $turnId) {
        $script:TurnId = [string]$turnId
        $script:TurnStatus = [string](Get-JsonProperty $turn 'status')
        Write-Control @{ turn_status = $script:TurnStatus }
    }
}

function Receive-Response($Socket, [int]$Id) {
    while ($true) {
        $message = Read-Message $Socket
        Update-FromNotification $message
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

function Get-FinalAgentText($Turn) {
    $text = $null
    $items = Get-JsonProperty $Turn 'items'
    if ($null -eq $items) {
        return $null
    }
    foreach ($item in $items) {
        $itemType = Get-JsonProperty $item 'type'
        $itemText = Get-JsonProperty $item 'text'
        if ($itemType -eq 'agentMessage' -and -not [string]::IsNullOrWhiteSpace([string]$itemText)) {
            $text = [string]$itemText
        }
    }
    return $text
}

$script:Endpoint = $null
$script:AppServerPid = $null
$script:ThreadId = $null
$script:TurnId = $null
$script:TurnStatus = 'starting'
$script:LastErrorText = $null
$script:LastAgentMessageText = $null
$script:LastFinalAgentMessageText = $null
$socket = $null
$appServer = $null

try {
    [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($ControlPath)) | Out-Null
    [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($EventsPath)) | Out-Null
    $port = Get-FreeTcpPort
    $script:Endpoint = "ws://127.0.0.1:$port"
    $healthUrl = "http://127.0.0.1:$port/healthz"

    $appArgs = @(
        'app-server',
        '--listen', $script:Endpoint,
        '-c', 'sandbox_mode="danger-full-access"',
        '-c', 'approval_policy="never"',
        '-c', ('mcp_servers.synapse.url=' + (ConvertTo-TomlStringLiteral $McpUrl)),
        '-c', 'mcp_servers.synapse.bearer_token_env_var="SYNAPSE_BEARER_TOKEN"'
    )
    if (-not [string]::IsNullOrWhiteSpace($Model)) {
        $appArgs += @('-c', ('model=' + (ConvertTo-TomlStringLiteral $Model)))
    }
    # The app-server transport is already the turn-completion observer for this
    # spawn. Do not pass the legacy Codex `notify` hook here: on Windows, its
    # TOML array is shell-fragile through npm shims and can prevent app-server
    # startup before any control artifact is usable.

    $launch = Get-CodexLaunchSpec -AppArgs $appArgs
    $appServer = Start-Process -FilePath $launch.File -ArgumentList $launch.Args -WindowStyle Hidden -RedirectStandardOutput $AppServerStdoutPath -RedirectStandardError $AppServerStderrPath -PassThru
    $script:AppServerPid = [int]$appServer.Id
    $script:TurnStatus = 'app_server_started'
    Write-Control @{}

    Wait-AppServerReady -Url $healthUrl -TimeoutMs 15000
    $socket = Connect-AppServer $script:Endpoint

    Send-WebSocketJson $socket ([ordered]@{
        id = 1
        method = 'initialize'
        params = [ordered]@{
            clientInfo = [ordered]@{ name = 'synapse-act-spawn-agent'; version = '0.1.0' }
            capabilities = [ordered]@{ experimentalApi = $true }
        }
    })
    [void](Receive-Response $socket 1)

    $threadParams = [ordered]@{
        cwd = $WorkingDir
        sandbox = 'danger-full-access'
        approvalPolicy = 'never'
        ephemeral = $true
        threadSource = 'subagent'
        sessionStartSource = 'startup'
        runtimeWorkspaceRoots = @($WorkingDir)
        config = [ordered]@{
            mcp_servers = [ordered]@{
                synapse = [ordered]@{
                    url = $McpUrl
                    bearer_token_env_var = 'SYNAPSE_BEARER_TOKEN'
                }
            }
        }
    }
    if (-not [string]::IsNullOrWhiteSpace($Model)) {
        $threadParams['model'] = $Model
    }
    Send-WebSocketJson $socket ([ordered]@{ id = 2; method = 'thread/start'; params = $threadParams })
    $threadResponse = Receive-Response $socket 2
    $threadResult = Get-JsonProperty $threadResponse 'result'
    $thread = Get-JsonProperty $threadResult 'thread'
    $script:ThreadId = [string](Get-JsonProperty $thread 'id')
    $script:TurnStatus = 'thread_started'
    Write-Control @{}

    $prompt = [string](Get-Content -Raw -LiteralPath $PromptPath -Encoding UTF8)
    $turnParams = [ordered]@{
        threadId = $script:ThreadId
        input = @([ordered]@{ type = 'text'; text = $prompt })
        cwd = $WorkingDir
        approvalPolicy = 'never'
        runtimeWorkspaceRoots = @($WorkingDir)
    }
    if (-not [string]::IsNullOrWhiteSpace($Model)) {
        $turnParams['model'] = $Model
    }
    $script:TurnStatus = 'turn_start_sending'
    Write-Control @{}
    Send-WebSocketJson $socket ([ordered]@{ id = 3; method = 'turn/start'; params = $turnParams })
    $script:TurnStatus = 'turn_start_sent'
    Write-Control @{}
    $turnResponse = Receive-Response $socket 3
    $turnResult = Get-JsonProperty $turnResponse 'result'
    $turn = Get-JsonProperty $turnResult 'turn'
    $script:TurnId = [string](Get-JsonProperty $turn 'id')
    $script:TurnStatus = [string](Get-JsonProperty $turn 'status')
    Write-Control @{}

    while ($true) {
        $message = Read-Message $socket
        Update-FromNotification $message
        $method = Get-JsonProperty $message 'method'
        $params = Get-JsonProperty $message 'params'
        $completedTurn = Get-JsonProperty $params 'turn'
        if ($method -eq 'turn/completed' -and [string](Get-JsonProperty $params 'threadId') -eq $script:ThreadId -and [string](Get-JsonProperty $completedTurn 'id') -eq $script:TurnId) {
            $script:TurnStatus = [string](Get-JsonProperty $completedTurn 'status')
            $finalText = Get-FinalAgentText $completedTurn
            if ([string]::IsNullOrWhiteSpace($finalText)) {
                $finalText = $script:LastFinalAgentMessageText
            }
            if ([string]::IsNullOrWhiteSpace($finalText)) {
                $finalText = $script:LastAgentMessageText
            }
            if ([string]::IsNullOrWhiteSpace($finalText)) {
                $finalText = ([ordered]@{
                    schema_version = 1
                    spawn_id = $SpawnId
                    cli = 'codex'
                    protocol = 'codex_app_server_ws'
                    status = $script:TurnStatus
                    thread_id = $script:ThreadId
                    turn_id = $script:TurnId
                    control_path = $ControlPath
                } | ConvertTo-Json -Depth 20)
            }
            Write-TextNoBom -Path $FinalMessagePath -Value $finalText
            Write-Control @{ turn_status = $script:TurnStatus }
            exit 0
        }
    }
} catch {
    $script:LastErrorText = $_.Exception.Message
    Write-Control @{ last_error = $script:LastErrorText; turn_status = 'runner_error' }
    Append-LineNoBom -Path $StderrPath -Value ("SYNAPSE_CODEX_APP_SERVER_RUNNER_ERROR: " + $script:LastErrorText)
    exit 1
} finally {
    if ($null -ne $socket) {
        try { $socket.Dispose() } catch {}
    }
    if ($null -ne $appServer -and -not $appServer.HasExited) {
        Stop-OwnedProcessTree -RootPid ([int]$appServer.Id)
    }
}
