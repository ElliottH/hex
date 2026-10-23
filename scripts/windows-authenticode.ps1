# Checks the Authenticode signatures CI applied through Azure Artifact
# Signing, then re-addresses the signed executables: signing changes their
# bytes, so each feed payload is rewritten with the signed executable's
# content-addressed name, size, and SHA-256 before the update feeds are
# signed. Run after both the executables and the installers are signed.
#
#   scripts/windows-authenticode.ps1 -Dist dist -Installers installers
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string]$Dist,
    [Parameter(Mandatory)] [string]$Installers
)
$ErrorActionPreference = 'Stop'

function Assert-Signed([string]$Path) {
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -ne 'Valid') {
        throw "$Path is not validly signed: $($signature.Status) $($signature.StatusMessage)"
    }
    # Artifact Signing certificates last days; only the timestamp keeps
    # the signature valid afterwards.
    if (-not $signature.TimeStamperCertificate) { throw "$Path has no RFC 3161 timestamp." }
    Write-Output "ok - $(Split-Path -Leaf $Path) signed by $($signature.SignerCertificate.Subject)"
}

$installers = @(Get-ChildItem -LiteralPath $Installers -Filter 'HEX-Setup-*.exe')
if ($installers.Count -eq 0) { throw "No installers in $Installers." }
foreach ($installer in $installers) { Assert-Signed $installer.FullName }

$payloads = @(Get-ChildItem -LiteralPath $Dist -Filter 'windows*-update.json.payload.json')
if ($payloads.Count -eq 0) { throw "No Windows feed payloads in $Dist." }
foreach ($file in $payloads) {
    $payload = Get-Content -Raw -LiteralPath $file.FullName | ConvertFrom-Json
    $executable = Join-Path $Dist $payload.artifact
    Assert-Signed $executable
    if ($payload.artifact -notmatch "^HEX-$([regex]::Escape($payload.version))-[0-9a-f]{64}-(.+)$") {
        throw "Unexpected artifact name $($payload.artifact)."
    }
    $suffix = $Matches[1]
    $sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
    $artifact = "HEX-$($payload.version)-$sha256-$suffix"
    if ($artifact -ne $payload.artifact) {
        Move-Item -LiteralPath $executable -Destination (Join-Path $Dist $artifact)
    }
    # The field order and compact form release-windows.sh writes.
    $signed = [ordered]@{
        schema_version = $payload.schema_version
        channel        = $payload.channel
        target         = $payload.target
        version        = $payload.version
        artifact       = $artifact
        bytes          = (Get-Item -LiteralPath (Join-Path $Dist $artifact)).Length
        sha256         = $sha256
    }
    [IO.File]::WriteAllText($file.FullName, ($signed | ConvertTo-Json -Compress) + "`n")
    Write-Output "ok - $($file.Name) names $artifact"
}
