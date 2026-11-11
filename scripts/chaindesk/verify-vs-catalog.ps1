param(
    [Parameter(Mandatory = $true)][string]$CatalogPath,
    [string]$VerifierAssembly = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\resources\app\ServiceHub\Services\Microsoft.VisualStudio.Setup.Service\Microsoft.VisualStudio.Setup.dll"
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)

# The installed Microsoft verifier requires .NET Framework's RSA provider.
if ($PSVersionTable.PSVersion.Major -ge 6) {
    & "$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe" -NoProfile -NonInteractive -File $PSCommandPath -CatalogPath $CatalogPath -VerifierAssembly $VerifierAssembly
    if ($LASTEXITCODE -ne 0) { throw 'Microsoft catalog signature verification failed' }
    return
}

$baseline = Get-Content -LiteralPath (Join-Path $PSScriptRoot '../../docs/engineeringBaseline.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$expected = $baseline.buildToolchain.acceptedCatalog
if (-not $expected) { throw 'No independently authenticated catalog is pinned' }
$catalogFile = Get-Item -LiteralPath $CatalogPath
if ($catalogFile.Length -ne $expected.size) { throw 'Catalog size mismatch' }
$stream = $catalogFile.OpenRead()
$hasher = [System.Security.Cryptography.SHA256]::Create()
try {
    $sha256 = [BitConverter]::ToString($hasher.ComputeHash($stream)).Replace('-', '').ToLowerInvariant()
} finally {
    $stream.Dispose()
    $hasher.Dispose()
}
if ($sha256 -ne $expected.sha256) { throw 'Catalog SHA-256 mismatch' }

$assemblySignature = Get-AuthenticodeSignature -LiteralPath $VerifierAssembly
if ($assemblySignature.Status.ToString() -ne 'Valid' -or $assemblySignature.SignerCertificate.Subject -ne $expected.signature.subject) {
    throw 'The catalog verifier must have a valid Microsoft Authenticode signature'
}
$assembly = [System.Reflection.Assembly]::LoadFrom((Get-Item -LiteralPath $VerifierAssembly).FullName)
$verifierType = $assembly.GetType('Microsoft.VisualStudio.Setup.Security.ManifestVerifier', $true)
$constructor = $verifierType.GetConstructor([System.Reflection.BindingFlags]'Public,NonPublic,Instance', $null, [type[]]@([System.IServiceProvider]), $null)
$verifier = $constructor.Invoke([object[]]@($null))
$stream = $catalogFile.OpenRead()
try {
    $verification = $verifier.Verify($stream, $catalogFile.FullName, $null)
} finally {
    $stream.Dispose()
}
if ($verification.Result.ToString() -ne 'Success' -or $verification.Signers[0] -ne $expected.signature.subject) {
    throw "Microsoft catalog verification failed: $($verification.Result)"
}

$catalog = Get-Content -LiteralPath $catalogFile.FullName -Raw -Encoding UTF8 | ConvertFrom-Json
$certificate = [System.Security.Cryptography.X509Certificates.X509Certificate2]::new([Convert]::FromBase64String($catalog.signature.keyInfo.x509Data[0]))
$chain = [System.Security.Cryptography.X509Certificates.X509Chain]::new()
try {
    if ($certificate.Thumbprint -ne $expected.signature.certificateThumbprint) { throw 'Catalog signer certificate mismatch' }
    foreach ($encoded in $catalog.signature.keyInfo.x509Data | Select-Object -Skip 1) {
        [void]$chain.ChainPolicy.ExtraStore.Add([System.Security.Cryptography.X509Certificates.X509Certificate2]::new([Convert]::FromBase64String($encoded)))
    }
    $chain.ChainPolicy.RevocationMode = [System.Security.Cryptography.X509Certificates.X509RevocationMode]::Online
    $chain.ChainPolicy.UrlRetrievalTimeout = [TimeSpan]::FromSeconds(15)
    [void]$chain.ChainPolicy.ApplicationPolicy.Add([System.Security.Cryptography.Oid]::new('1.3.6.1.5.5.7.3.3'))
    if (-not $chain.Build($certificate)) { throw "Catalog certificate chain failed: $($chain.ChainStatus.Status)" }
    $rootThumbprint = $chain.ChainElements[$chain.ChainElements.Count - 1].Certificate.Thumbprint
    if ($rootThumbprint -ne $expected.signature.rootCertificateThumbprint) { throw 'Catalog root certificate mismatch' }
} finally {
    $chain.Dispose()
    $certificate.Dispose()
}
if ($catalog.info.buildVersion -ne $baseline.buildToolchain.buildVersion) { throw 'Visual Studio build version mismatch' }
foreach ($package in $baseline.buildToolchain.packages) {
    $matches = @($catalog.packages | Where-Object { $_.id -eq $package.id -and $_.version -eq $package.version })
    if ($matches.Count -ne 1) { throw "Pinned package missing or ambiguous: $($package.id)" }
}

[ordered]@{
    visualStudioVersion = $baseline.buildToolchain.visualStudioVersion
    buildVersion = $catalog.info.buildVersion
    sha256 = $sha256
    size = $catalogFile.Length
    signatureResult = $verification.Result.ToString()
    signers = @($verification.Signers)
    verifierAssemblyVersion = $assembly.GetName().Version.ToString()
    verifierAuthenticode = $assemblySignature.Status.ToString()
    onlineCertificateChain = 'Valid'
    rootCertificateThumbprint = $rootThumbprint
    channelMetadataMatched = $false
    trustBasis = 'pinnedBytesAndMicrosoftEmbeddedSignature'
} | ConvertTo-Json -Depth 4
