param(
    [Parameter(Mandatory)][string]$ModuleCache,
    [Parameter(Mandatory)][string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
$apkRoot = Split-Path $PSScriptRoot -Parent
$patches = Get-Content -LiteralPath (Join-Path $apkRoot 'core/patches/android-a0.json') -Encoding UTF8 -Raw | ConvertFrom-Json
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$sourceRoot = Join-Path $OutputDirectory ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $sourceRoot | Out-Null
$modules = @{}
foreach ($patch in $patches.files) {
    $source = Join-Path $ModuleCache $patch.modulePath
    if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant() -ne $patch.sourceSha256) {
        throw "Pinned source mismatch for $($patch.modulePath)"
    }
    $content = [IO.File]::ReadAllText($source, [Text.Encoding]::UTF8)
    foreach ($edit in $patch.edits) {
        if ([regex]::Matches($content, [regex]::Escape($edit.before)).Count -ne 1) { throw "Patch context is not unique: $($patch.modulePath)" }
        $content = $content.Replace($edit.before, $edit.after)
    }
    if ($patch.modulePath -notmatch '^(.+@[^/]+)/(.+)$') { throw 'Invalid pinned module path' }
    $moduleVersion = $Matches[1]
    $relativeFile = $Matches[2]
    if (-not $modules.ContainsKey($moduleVersion)) {
        $moduleTarget = Join-Path $sourceRoot ($moduleVersion.Split('/')[-1])
        Copy-Item -LiteralPath (Join-Path $ModuleCache $moduleVersion) -Destination $moduleTarget -Recurse
        $modules[$moduleVersion] = $moduleTarget
    }
    $target = Join-Path $modules[$moduleVersion] $relativeFile
    [IO.File]::SetAttributes($target, ([IO.File]::GetAttributes($target) -band (-bnot [IO.FileAttributes]::ReadOnly)))
    [IO.File]::WriteAllText($target, $content, [Text.UTF8Encoding]::new($false))
}
$moduleFile = Join-Path $sourceRoot 'go.mod'
$moduleText = Get-Content -LiteralPath (Join-Path $apkRoot 'core/go.mod') -Encoding UTF8 -Raw
foreach ($moduleVersion in $modules.Keys) {
    $moduleName = $moduleVersion.Substring(0, $moduleVersion.IndexOf('@'))
    $modulePath = $modules[$moduleVersion].Replace('\', '/')
    $moduleText += "`nreplace $moduleName => `"$modulePath`"`n"
}
[IO.File]::WriteAllText($moduleFile, $moduleText, [Text.UTF8Encoding]::new($false))
Copy-Item -LiteralPath (Join-Path $apkRoot 'core/go.sum') -Destination (Join-Path $sourceRoot 'go.sum')
Write-Output $moduleFile
