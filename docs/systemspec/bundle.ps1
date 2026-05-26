# bundle.ps1 — concatenate the systemspec docs into one .md file.
#
# Output:   docs/systemspec/SYNAPSE_SYSTEMSPEC.md
# Usage:    pwsh ./docs/systemspec/bundle.ps1   (from repo root)
#           pwsh ./bundle.ps1                   (from docs/systemspec/)
#
# Order:    README.md, then 01..15.
# Rewrites: in-doc cross-references like [text](04_storage_layer.md[#frag])
#           are rewritten to in-document anchors so the bundle is self-linking.
#           Links to non-bundled .md files (impplan/computergames/adr/etc.)
#           keep their original target.

$ErrorActionPreference = 'Stop'

$Here    = Split-Path -Parent $MyInvocation.MyCommand.Definition
$OutFile = Join-Path $Here 'SYNAPSE_SYSTEMSPEC.md'

# Ordered file list. Anchor strings double as section ids in the TOC.
$Dash = [char]0x2014
$Files = @(
    [pscustomobject]@{ Name = 'README.md';                       Anchor = 'index';   Title = 'Index (README)' },
    [pscustomobject]@{ Name = '01_system_overview.md';           Anchor = 'file-01'; Title = "01 $Dash System Overview" },
    [pscustomobject]@{ Name = '02_source_code_map.md';           Anchor = 'file-02'; Title = "02 $Dash Source Code Map" },
    [pscustomobject]@{ Name = '03_configuration.md';             Anchor = 'file-03'; Title = "03 $Dash Configuration" },
    [pscustomobject]@{ Name = '04_storage_layer.md';             Anchor = 'file-04'; Title = "04 $Dash Storage Layer" },
    [pscustomobject]@{ Name = '05_core_types_and_errors.md';     Anchor = 'file-05'; Title = "05 $Dash Core Types and Errors" },
    [pscustomobject]@{ Name = '06_mcp_service_and_transports.md'; Anchor = 'file-06'; Title = "06 $Dash MCP Service and Transports" },
    [pscustomobject]@{ Name = '07_reflex_runtime.md';            Anchor = 'file-07'; Title = "07 $Dash Reflex Runtime" },
    [pscustomobject]@{ Name = '08_action_subsystem.md';          Anchor = 'file-08'; Title = "08 $Dash Action Subsystem" },
    [pscustomobject]@{ Name = '09_perception_and_capture.md';    Anchor = 'file-09'; Title = "09 $Dash Perception and Capture" },
    [pscustomobject]@{ Name = '10_audio_and_models.md';          Anchor = 'file-10'; Title = "10 $Dash Audio and Models" },
    [pscustomobject]@{ Name = '11_profiles_hid_telemetry.md';    Anchor = 'file-11'; Title = "11 $Dash Profiles, HID, Telemetry, Test Utils" },
    [pscustomobject]@{ Name = '12_milestones_and_roadmap.md';    Anchor = 'file-12'; Title = "12 $Dash Milestones and Roadmap" },
    [pscustomobject]@{ Name = '13_mcp_tool_reference.md';        Anchor = 'file-13'; Title = "13 $Dash MCP Tool Reference" },
    [pscustomobject]@{ Name = '14_test_suite.md';                Anchor = 'file-14'; Title = "14 $Dash Test Suite" },
    [pscustomobject]@{ Name = '15_verification_report.md';       Anchor = 'file-15'; Title = "15 $Dash Verification Report" }
)

# filename -> anchor lookup for link rewriting
$AnchorMap = @{}
foreach ($f in $Files) { $AnchorMap[$f.Name] = $f.Anchor }

# Pre-flight: every source file must exist
foreach ($f in $Files) {
    $p = Join-Path $Here $f.Name
    if (-not (Test-Path $p)) { throw "Missing source file: $p" }
}

# Build the bundle
$Today = (Get-Date).ToString('yyyy-MM-dd')
$Out   = [System.Text.StringBuilder]::new()

[void]$Out.AppendLine("# Synapse Systemspec $Dash Bundled Reference")
[void]$Out.AppendLine()
[void]$Out.AppendLine("> Auto-generated $Today by ``docs/systemspec/bundle.ps1``. Source: the 16 individual ``docs/systemspec/*.md`` files, concatenated in order. In-bundle cross-references between systemspec files are rewritten to anchors; references to files outside the bundle (impplan, computergames, adr, source code) keep their original paths.")
[void]$Out.AppendLine('>')
[void]$Out.AppendLine('> Re-run the script after editing any source file so the bundle stays in sync. The individual files remain the authoritative copies.')
[void]$Out.AppendLine()
[void]$Out.AppendLine('## Bundle table of contents')
[void]$Out.AppendLine()
foreach ($f in $Files) {
    [void]$Out.AppendLine("- [$($f.Title)](#$($f.Anchor))")
}
[void]$Out.AppendLine()

foreach ($f in $Files) {
    $path = Join-Path $Here $f.Name
    $body = Get-Content -LiteralPath $path -Raw -Encoding UTF8

    # Rewrite [text](<systemspec-file>.md[#frag]) -> [text](#<anchor>).
    # Matches each known systemspec filename; non-systemspec .md links are untouched.
    foreach ($k in $AnchorMap.Keys) {
        $escaped = [Regex]::Escape($k)
        $anchor  = $AnchorMap[$k]
        $pattern = "\]\(" + $escaped + "(?:#[^)]*)?\)"
        $body    = [Regex]::Replace($body, $pattern, "](#$anchor)")
    }

    [void]$Out.AppendLine()
    [void]$Out.AppendLine('---')
    [void]$Out.AppendLine()
    [void]$Out.AppendLine("<a id=""$($f.Anchor)""></a>")
    [void]$Out.AppendLine()
    [void]$Out.AppendLine("> Source: ``docs/systemspec/$($f.Name)``")
    [void]$Out.AppendLine()
    [void]$Out.AppendLine($body.TrimEnd())
    [void]$Out.AppendLine()
}

# Write UTF-8 (no BOM) so GitHub-flavored markdown renders cleanly
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText($OutFile, $Out.ToString(), $utf8NoBom)

$size = (Get-Item -LiteralPath $OutFile).Length
Write-Host "Wrote $OutFile ($size bytes, $($Files.Count) source files)"
