param(
    [string]$SynapseNativeHostExe = "$env:USERPROFILE\.cargo\bin\synapse-chrome-native-host.exe",
    [string]$ExtensionId = "leoocgnkjnplbfdbklajepahofecgfbk"
)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$extensionDir = Join-Path $repoRoot 'extensions\synapse-chrome-debugger'
$manifestPath = Join-Path $extensionDir 'manifest.json'
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw "SYNAPSE_CHROME_EXTENSION_MANIFEST_MISSING path=$manifestPath"
}
$extensionManifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
$requiredPermissions = @($extensionManifest.permissions)
$optionalPermissions = @($extensionManifest.optional_permissions)
$hostPermissions = @($extensionManifest.host_permissions)
if ($requiredPermissions -contains 'debugger') {
    throw "SYNAPSE_CHROME_EXTENSION_REQUIRED_DEBUGGER_PERMISSION_FORBIDDEN path=$manifestPath remediation=normal end-user bridge must use chrome.tabs without required debugger permission"
}
if ($optionalPermissions -contains 'debugger') {
    throw "SYNAPSE_CHROME_EXTENSION_OPTIONAL_DEBUGGER_PERMISSION_FORBIDDEN path=$manifestPath remediation=normal end-user bridge must use chrome.tabs without optional debugger permission; use raw CDP from a Synapse-launched automation profile for DOM/action CDP"
}
if ($requiredPermissions -contains 'nativeMessaging') {
    throw "SYNAPSE_CHROME_EXTENSION_NATIVE_MESSAGING_FORBIDDEN path=$manifestPath remediation=normal end-user bridge must use direct localhost HTTP registration plus WebSocket command delivery; nativeMessaging can launch a visible cmd.exe wrapper on Windows"
}
if ($optionalPermissions -contains 'nativeMessaging') {
    throw "SYNAPSE_CHROME_EXTENSION_OPTIONAL_NATIVE_MESSAGING_FORBIDDEN path=$manifestPath remediation=normal end-user bridge must not request nativeMessaging"
}
if ($hostPermissions -notcontains 'http://127.0.0.1:7700/*') {
    throw "SYNAPSE_CHROME_EXTENSION_LOCALHOST_PERMISSION_MISSING path=$manifestPath remediation=normal bridge requires host_permissions http://127.0.0.1:7700/* for direct daemon registration and message posting"
}

$nativeRoot = Join-Path $env:APPDATA 'synapse\chrome-debugger'
New-Item -ItemType Directory -Force -Path $nativeRoot | Out-Null

$hostName = 'com.synapse.chrome_debugger'
$hostManifestPath = Join-Path $nativeRoot "$hostName.json"
$registryPath = "HKCU:\Software\Google\Chrome\NativeMessagingHosts\$hostName"
if (Test-Path -LiteralPath $registryPath) {
    Remove-Item -LiteralPath $registryPath -Force
}
if (Test-Path -LiteralPath $registryPath) {
    throw "SYNAPSE_CHROME_NATIVE_HOST_REGISTRY_REMOVE_FAILED path=$registryPath remediation=normal bridge must not leave a nativeMessaging host registered because Chrome may launch cmd.exe as an intermediary"
}
if (Test-Path -LiteralPath $hostManifestPath -PathType Leaf) {
    Remove-Item -LiteralPath $hostManifestPath -Force
}
if (Test-Path -LiteralPath $hostManifestPath -PathType Leaf) {
    throw "SYNAPSE_CHROME_NATIVE_HOST_MANIFEST_REMOVE_FAILED path=$hostManifestPath remediation=normal bridge must use direct localhost WebSocket command delivery only"
}

$chromeProcesses = @(Get-CimInstance Win32_Process -Filter "Name='chrome.exe'" -ErrorAction SilentlyContinue | ForEach-Object {
    $commandLine = [string]$_.CommandLine
    [pscustomobject]@{
        pid = [int]$_.ProcessId
        command_line_readable = -not [string]::IsNullOrWhiteSpace($commandLine)
        has_silent_debugger_switch = $commandLine -match '(^|\s)--silent-debugger-extension-api(\s|=|$)'
    }
})

$chromeUserDataRoot = Join-Path $env:LOCALAPPDATA 'Google\Chrome\User Data'
$synapseChromeProfileReadback = @()
$staleSynapseActivePermissions = @()
$externalDebuggerOrNativeExtensions = @()
if (Test-Path -LiteralPath $chromeUserDataRoot -PathType Container) {
    $profileDirs = @(Get-ChildItem -LiteralPath $chromeUserDataRoot -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -ne 'Snapshots' })
    foreach ($profileDir in $profileDirs) {
        foreach ($prefFileName in @('Secure Preferences', 'Preferences')) {
            $prefPath = Join-Path $profileDir.FullName $prefFileName
            if (-not (Test-Path -LiteralPath $prefPath -PathType Leaf)) {
                continue
            }
            try {
                $pref = Get-Content -Raw -LiteralPath $prefPath | ConvertFrom-Json -ErrorAction Stop
            } catch {
                $synapseChromeProfileReadback += [pscustomobject]@{
                    profile = $profileDir.Name
                    pref_file = $prefFileName
                    path = $prefPath
                    parse_error = $_.Exception.Message
                }
                continue
            }
            if (-not $pref.extensions -or -not $pref.extensions.settings) {
                continue
            }
            foreach ($extensionProperty in $pref.extensions.settings.PSObject.Properties) {
                $setting = $extensionProperty.Value
                $activeApi = @()
                if ($setting.active_permissions -and $setting.active_permissions.api) {
                    $activeApi = @($setting.active_permissions.api)
                }
                $grantedApi = @()
                if ($setting.granted_permissions -and $setting.granted_permissions.api) {
                    $grantedApi = @($setting.granted_permissions.api)
                }
                if ($extensionProperty.Name -eq $ExtensionId) {
                    $row = [pscustomobject]@{
                        profile = $profileDir.Name
                        pref_file = $prefFileName
                        path = $prefPath
                        manifest_path = $setting.path
                        active_api = $activeApi
                        granted_api = $grantedApi
                    }
                    $synapseChromeProfileReadback += $row
                    if ($activeApi -contains 'debugger' -or $activeApi -contains 'nativeMessaging') {
                        $staleSynapseActivePermissions += $row
                    }
                } elseif ($activeApi -contains 'debugger' -or $activeApi -contains 'nativeMessaging') {
                    $externalDebuggerOrNativeExtensions += [pscustomobject]@{
                        profile = $profileDir.Name
                        pref_file = $prefFileName
                        extension_id = $extensionProperty.Name
                        name = $setting.manifest.name
                        location = $setting.location
                        manifest_path = $setting.path
                        active_api = $activeApi
                    }
                }
            }
        }
    }
}
if ($staleSynapseActivePermissions.Count -gt 0) {
    $detail = $staleSynapseActivePermissions | ConvertTo-Json -Depth 6 -Compress
    throw "SYNAPSE_CHROME_EXTENSION_STALE_ACTIVE_DEBUGGER_PERMISSION extension_id=$ExtensionId detail=$detail remediation=reload the unpacked Synapse Chrome Bridge from chrome://extensions or remove/re-add it; the normal bridge must be active with tabs only before setup can pass"
}

$externalNativeMessagingProcesses = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
    Where-Object {
        $_.CommandLine -match 'chrome\.nativeMessaging' -and
        $_.CommandLine -notmatch [regex]::Escape($ExtensionId)
    } |
    Select-Object ProcessId, ParentProcessId, Name, ExecutablePath, CommandLine)

[pscustomobject]@{
    ok = $true
    native_host = $hostName
    native_manifest = $null
    registry_key = $registryPath
    binary = $null
    extension_id = $ExtensionId
    extension_dir = $extensionDir
    daemon_bridge_transport = 'direct_localhost_websocket'
    daemon_bridge_origin = "chrome-extension://$ExtensionId"
    background_navigation_backend = 'chrome.tabs_no_debugger_permission_no_native_messaging'
    reconnect_driver = 'chrome.alarms_30s_direct_localhost_register'
    attach_popup_prevention = 'normal_bridge_tabs_only_no_debugger_api_no_nativeMessaging_permission_plus_daemon_side_attach_disabled'
    normal_bridge_attach_commands_available = $false
    normal_bridge_debugger_api_calls_present = $false
    required_alarms_permission_present = ($requiredPermissions -contains 'alarms')
    required_debugger_permission_present = $false
    optional_debugger_permission_present = $false
    required_native_messaging_permission_present = $false
    optional_native_messaging_permission_present = $false
    localhost_host_permission_present = $true
    native_host_registry_present = (Test-Path -LiteralPath $registryPath)
    native_host_manifest_present = (Test-Path -LiteralPath $hostManifestPath)
    silent_debugger_switch_required_for_attach_commands = $false
    silent_debugger_switch = $null
    current_chrome_processes = $chromeProcesses
    synapse_chrome_profile_readback = $synapseChromeProfileReadback
    external_debugger_or_native_extensions = $externalDebuggerOrNativeExtensions
    external_native_messaging_processes = $externalNativeMessagingProcesses
}
