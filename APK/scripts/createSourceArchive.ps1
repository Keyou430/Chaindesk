param([Parameter(Mandatory)][string]$OutputDirectory)

$ErrorActionPreference = 'Stop'
$apkRoot = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
$repoRoot = Split-Path $apkRoot -Parent
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$outputRoot = (Resolve-Path -LiteralPath $OutputDirectory).Path
$archivePath = Join-Path $outputRoot 'chaindesk-android-source.zip'
if (Test-Path -LiteralPath $archivePath) { throw 'Source archive already exists; choose a new output directory' }
$sourceFiles = [Collections.Generic.List[IO.FileInfo]]::new()
$pending = [Collections.Generic.Queue[IO.DirectoryInfo]]::new()
$pending.Enqueue([IO.DirectoryInfo]::new($apkRoot))
while ($pending.Count) {
    $directory = $pending.Dequeue()
    foreach ($child in $directory.GetDirectories()) {
        if ($child.Name -notin @('dist', 'build', '.gradle', '.kotlin', '.cxx') -and -not ($child.Attributes -band [IO.FileAttributes]::ReparsePoint)) { $pending.Enqueue($child) }
    }
    foreach ($file in $directory.GetFiles()) {
        $relative = [IO.Path]::GetRelativePath($apkRoot, $file.FullName).Replace('\', '/')
        if ($relative -ne 'android/local.properties' -and $relative -notmatch '\.(jks|keystore)$' -and -not ($file.Attributes -band [IO.FileAttributes]::ReparsePoint)) { $sourceFiles.Add($file) }
    }
}
$sourceFiles = $sourceFiles | Sort-Object FullName
$sourceFiles = @($sourceFiles) + (Get-Item -LiteralPath (Join-Path $repoRoot 'LICENSE'))
$entries = @()
$stream = [IO.File]::Open($archivePath, [IO.FileMode]::CreateNew)
$archive = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($file in $sourceFiles) {
        $relative = [IO.Path]::GetRelativePath($repoRoot, $file.FullName).Replace('\', '/')
        $bytes = [IO.File]::ReadAllBytes($file.FullName)
        $entry = $archive.CreateEntry($relative, [IO.Compression.CompressionLevel]::Optimal)
        $entryStream = $entry.Open()
        try { $entryStream.Write($bytes, 0, $bytes.Length) } finally { $entryStream.Dispose() }
        $entries += [ordered]@{ path = $relative; bytes = $bytes.Length; sha256 = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant() }
    }
} finally { $archive.Dispose(); $stream.Dispose() }
[ordered]@{ schemaVersion = 1; files = $entries } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $outputRoot 'source-inputs.json') -Encoding UTF8
Write-Output $archivePath
