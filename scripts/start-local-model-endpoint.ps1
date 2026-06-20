param(
    [string]$Python = "/home/cabdru/vllm-venv/bin/python",
    [string]$ModelDir = "/home/cabdru/subconscious/merged_qwen8v2",
    [string]$ModelName = "qwen8v2-tool",
    [int]$Port = 8002,
    [string]$LogPath = "/home/cabdru/subconscious/issue1281_qwen8v2_8002.log",
    [string]$PidPath = "/home/cabdru/subconscious/issue1281_qwen8v2_8002.pid"
)

$repoScript = "/mnt/c/code/Synapse/scripts/local-model-openai-chat.py"
$existingListener = wsl.exe -e bash -lc "ss -ltn '( sport = :$Port )' | tail -n +2"
if (-not [string]::IsNullOrWhiteSpace($existingListener)) {
    Write-Output "LOCAL_MODEL_ENDPOINT_ALREADY_LISTENING port=$Port"
    exit 0
}

$bash = @"
set -euo pipefail
rm -f "$PidPath"
if ss -ltn "( sport = :$Port )" | grep -q ":$Port"; then
  echo "LOCAL_MODEL_ENDPOINT_ALREADY_LISTENING port=$Port"
  exit 0
fi
test -x "$Python"
test -d "$ModelDir"
test -f "$repoScript"
echo "`$`$" > "$PidPath"
exec "$Python" "$repoScript" --model-dir "$ModelDir" --model-name "$ModelName" --host 127.0.0.1 --port "$Port" > "$LogPath" 2>&1
"@

$encoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($bash))
$argumentLine = "-e bash -lc ""echo $encoded | base64 -d | bash"""
$proc = Start-Process -FilePath "wsl.exe" `
    -ArgumentList $argumentLine `
    -WindowStyle Hidden `
    -PassThru
for ($attempt = 0; $attempt -lt 40; $attempt++) {
    Start-Sleep -Milliseconds 250
    $wslPid = wsl.exe -e bash -lc "cat '$PidPath' 2>/dev/null || true"
    if (-not [string]::IsNullOrWhiteSpace($wslPid)) {
        break
    }
    if ($proc.HasExited) {
        break
    }
}
if ([string]::IsNullOrWhiteSpace($wslPid)) {
    throw "LOCAL_MODEL_ENDPOINT_START_FAILED pidfile_missing path=$PidPath windows_pid=$($proc.Id)"
}
Write-Output "LOCAL_MODEL_ENDPOINT_STARTED windows_pid=$($proc.Id) wsl_pid=$($wslPid.Trim()) port=$Port log=$LogPath"
