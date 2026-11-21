param(
    [string]$CacheRoot = (Join-Path $env:USERPROFILE '.cache/chaindesk-android')
)

$ErrorActionPreference = 'Stop'
$assetRoot = Join-Path $PSScriptRoot '../android/app/src/main/assets/rules'
$manifest = Get-Content -LiteralPath (Join-Path $assetRoot 'manifest.json') -Encoding UTF8 -Raw | ConvertFrom-Json
$downloadRoot = Join-Path $CacheRoot "rules/$($manifest.artifactCommit)"
New-Item -ItemType Directory -Path $downloadRoot -Force | Out-Null

function Assert-RuleAsset([string]$Path, $Entry) {
    if ((Get-Item -LiteralPath $Path).Length -ne $Entry.bytes -or
        (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $Entry.sha256) {
        throw "Pinned rule asset mismatch: $($Entry.name)"
    }
}

foreach ($entry in @($manifest.files) + @($manifest.licenses)) {
    $cached = Join-Path $downloadRoot $entry.name
    if (-not (Test-Path -LiteralPath $cached)) {
        $partial = "$cached.part"
        Invoke-WebRequest -Uri $entry.url -OutFile $partial -MaximumRedirection 3 -TimeoutSec 300
        Assert-RuleAsset $partial $entry
        Move-Item -LiteralPath $partial -Destination $cached
    }
    Assert-RuleAsset $cached $entry
}

foreach ($entry in @($manifest.files) + @($manifest.licenses)) {
    $destination = Join-Path $assetRoot $entry.name
    Copy-Item -LiteralPath (Join-Path $downloadRoot $entry.name) -Destination $destination -Force
    Assert-RuleAsset $destination $entry
}

Write-Output "Verified bundled rules: $($manifest.ruleVersion)"
