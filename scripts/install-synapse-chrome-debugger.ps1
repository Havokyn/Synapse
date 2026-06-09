param(
    [string]$SynapseMcpExe = "$env:USERPROFILE\.cargo\bin\synapse-mcp.exe",
    [string]$ExtensionId = "leoocgnkjnplbfdbklajepahofecgfbk"
)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$extensionDir = Join-Path $repoRoot 'extensions\synapse-chrome-debugger'
$manifestPath = Join-Path $extensionDir 'manifest.json'
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw "SYNAPSE_CHROME_EXTENSION_MANIFEST_MISSING path=$manifestPath"
}
if (-not (Test-Path -LiteralPath $SynapseMcpExe -PathType Leaf)) {
    throw "SYNAPSE_CHROME_NATIVE_HOST_BINARY_MISSING path=$SynapseMcpExe remediation=build/install synapse-mcp first"
}

$nativeRoot = Join-Path $env:APPDATA 'synapse\chrome-debugger'
New-Item -ItemType Directory -Force -Path $nativeRoot | Out-Null

$hostName = 'com.synapse.chrome_debugger'
$hostManifestPath = Join-Path $nativeRoot "$hostName.json"
$hostManifest = [ordered]@{
    name = $hostName
    description = 'Synapse Chrome debugger native-messaging bridge'
    path = (Resolve-Path -LiteralPath $SynapseMcpExe).Path
    type = 'stdio'
    allowed_origins = @("chrome-extension://$ExtensionId/")
}
$hostManifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $hostManifestPath -Encoding UTF8

$registryPath = "HKCU:\Software\Google\Chrome\NativeMessagingHosts\$hostName"
$registrySubKey = "Software\Google\Chrome\NativeMessagingHosts\$hostName"
New-Item -Path $registryPath -Force | Out-Null
$registryKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($registrySubKey, $true)
if ($null -eq $registryKey) {
    throw "SYNAPSE_CHROME_NATIVE_HOST_REGISTRY_OPEN_FAILED path=$registryPath"
}
try {
    $registryKey.SetValue('', $hostManifestPath, [Microsoft.Win32.RegistryValueKind]::String)
}
finally {
    $registryKey.Dispose()
}

$readbackManifest = Get-Content -Raw -LiteralPath $hostManifestPath | ConvertFrom-Json
$readbackRegistryKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($registrySubKey, $false)
if ($null -eq $readbackRegistryKey) {
    throw "SYNAPSE_CHROME_NATIVE_HOST_REGISTRY_READ_FAILED path=$registryPath"
}
try {
    $readbackRegistry = $readbackRegistryKey.GetValue('')
}
finally {
    $readbackRegistryKey.Dispose()
}

if ($readbackRegistry -ne $hostManifestPath) {
    throw "SYNAPSE_CHROME_NATIVE_HOST_REGISTRY_MISMATCH expected=$hostManifestPath actual=$readbackRegistry"
}
if ($readbackManifest.name -ne $hostName) {
    throw "SYNAPSE_CHROME_NATIVE_HOST_NAME_MISMATCH expected=$hostName actual=$($readbackManifest.name)"
}
if ($readbackManifest.allowed_origins[0] -ne "chrome-extension://$ExtensionId/") {
    throw "SYNAPSE_CHROME_NATIVE_HOST_ORIGIN_MISMATCH expected=chrome-extension://$ExtensionId/ actual=$($readbackManifest.allowed_origins[0])"
}

[pscustomobject]@{
    ok = $true
    native_host = $hostName
    native_manifest = $hostManifestPath
    registry_key = $registryPath
    binary = $readbackManifest.path
    extension_id = $ExtensionId
    extension_dir = $extensionDir
}
