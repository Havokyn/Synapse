param(
    [Parameter(Mandatory = $true)]
    [string]$BaselineJson,

    [Parameter(Mandatory = $true)]
    [string]$CandidateJson,

    [double]$MaxRegressionPercent = 20.0,

    [string[]]$Bench = @()
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Read-CritcmpExport {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $resolved = Resolve-Path -LiteralPath $Path
    $doc = Get-Content -Raw -LiteralPath $resolved | ConvertFrom-Json
    if ($null -eq $doc.benchmarks) {
        throw "$resolved is not a critcmp export: missing benchmarks object"
    }

    $benches = @{}
    foreach ($property in $doc.benchmarks.PSObject.Properties) {
        $estimate = $property.Value.criterion_estimates_v1
        if ($null -eq $estimate -or $null -eq $estimate.mean -or $null -eq $estimate.mean.point_estimate) {
            throw "$resolved benchmark '$($property.Name)' is missing criterion_estimates_v1.mean.point_estimate"
        }

        $meanNs = [double]$estimate.mean.point_estimate
        if ($meanNs -le 0.0) {
            throw "$resolved benchmark '$($property.Name)' has non-positive mean point estimate '$meanNs'"
        }

        $benches[$property.Name] = $meanNs
    }

    return ,$benches
}

$baseline = Read-CritcmpExport -Path $BaselineJson
$candidate = Read-CritcmpExport -Path $CandidateJson

if ($Bench.Count -gt 0) {
    $benchNames = $Bench
} else {
    $benchNames = @($baseline.Keys | Sort-Object)
}

if ($benchNames.Count -eq 0) {
    throw "no benchmarks found in baseline export"
}

$failed = $false
$epsilon = 0.000000001

foreach ($benchName in $benchNames) {
    if (-not $baseline.ContainsKey($benchName)) {
        Write-Output "source_of_truth=bench_delta bench=""$benchName"" status=FAIL reason=missing_from_baseline"
        $failed = $true
        continue
    }

    if (-not $candidate.ContainsKey($benchName)) {
        Write-Output "source_of_truth=bench_delta bench=""$benchName"" status=FAIL reason=missing_from_candidate"
        $failed = $true
        continue
    }

    $baselineNs = $baseline[$benchName]
    $candidateNs = $candidate[$benchName]
    $deltaPercent = (($candidateNs - $baselineNs) / $baselineNs) * 100.0
    $status = "PASS"
    if ($deltaPercent -gt ($MaxRegressionPercent + $epsilon)) {
        $status = "FAIL"
        $failed = $true
    }

    Write-Output (
        "source_of_truth=bench_delta bench=""{0}"" baseline_ns={1:F3} candidate_ns={2:F3} delta_percent={3:F3} threshold_percent={4:F3} status={5}" -f
        $benchName,
        $baselineNs,
        $candidateNs,
        $deltaPercent,
        $MaxRegressionPercent,
        $status
    )
}

if ($failed) {
    exit 1
}
