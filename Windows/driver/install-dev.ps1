#Requires -RunAsAdministrator
<#
.SYNOPSIS
    Test-signs and installs (or uninstalls) the opendisplay-idd driver
    package for local development iteration.

.DESCRIPTION
    Dev-only install path (T8). Requires `bcdedit /set testsigning on`
    (and a reboot) once per dev machine — this script does not flip that
    setting itself, since it's a machine-wide change the developer should
    make deliberately.

    ############################################################################
    # PRODUCTION SIGNING SWAP POINT
    #
    # Everything between "test-sign the catalog" and "install the package"
    # below is what a release build replaces: instead of Inf2Cat + a local
    # test certificate, the release pipeline submits the same Inf2Cat
    # output to Microsoft Partner Center for attestation signing
    # (spec.md Assumptions: driver signing) and receives back a
    # Microsoft-signed opendisplay-idd.cat, which pnputil then installs
    # exactly as this script does — no testsigning mode, no reboot, no
    # Secure Boot changes (spec.md AC: driver install story, AC 5).
    ############################################################################

.PARAMETER Uninstall
    Removes the driver package instead of installing it.

.PARAMETER DriverDir
    Directory containing opendisplay-idd.inf and its built .dll/.cat.
    Defaults to this script's own directory.
#>
[CmdletBinding()]
param(
    [switch]$Uninstall,
    [string]$DriverDir = $PSScriptRoot
)

$ErrorActionPreference = 'Stop'

$infPath = Join-Path $DriverDir 'opendisplay-idd.inf'
$certSubject = 'CN=OpenDisplay Dev Test Certificate'
$certStoreLocation = 'Cert:\LocalMachine\My'

function Assert-DriverFilesExist {
    if (-not (Test-Path $infPath)) {
        throw "opendisplay-idd.inf not found at '$infPath'. Build the driver project first."
    }
}

function Get-OrCreateDevCertificate {
    $existing = Get-ChildItem $certStoreLocation | Where-Object { $_.Subject -eq $certSubject }
    if ($existing) {
        return $existing[0]
    }

    Write-Host "Creating a new local test-signing certificate ($certSubject)..."
    return New-SelfSignedCertificate `
        -Type Custom `
        -Subject $certSubject `
        -KeyUsage DigitalSignature `
        -FriendlyName 'OpenDisplay Dev Test Certificate' `
        -CertStoreLocation $certStoreLocation `
        -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
}

function Install-DriverPackage {
    Assert-DriverFilesExist

    # --- Test-sign the catalog (dev only; see the signing-swap-point note
    #     above for what release builds do instead). ---
    $cert = Get-OrCreateDevCertificate
    $catPath = Join-Path $DriverDir 'opendisplay-idd.cat'

    Write-Host 'Generating the driver catalog (Inf2Cat)...'
    & Inf2Cat.exe /driver:"$DriverDir" /os:10_X64 /verbose
    if ($LASTEXITCODE -ne 0) {
        throw "Inf2Cat failed with exit code $LASTEXITCODE"
    }

    Write-Host 'Test-signing the catalog (signtool)...'
    & signtool.exe sign /v /s 'My' /sha1 $cert.Thumbprint /fd sha256 "$catPath"
    if ($LASTEXITCODE -ne 0) {
        throw "signtool failed with exit code $LASTEXITCODE"
    }

    # Trust the dev cert as a local root so Windows accepts the test
    # signature without additional prompts, matching the "single
    # elevation prompt" bar this script itself already runs under.
    $trustedRoot = Get-ChildItem 'Cert:\LocalMachine\Root' | Where-Object { $_.Thumbprint -eq $cert.Thumbprint }
    if (-not $trustedRoot) {
        Write-Host 'Adding the dev certificate to the local machine trusted root store...'
        $rootStore = New-Object System.Security.Cryptography.X509Certificates.X509Store('Root', 'LocalMachine')
        $rootStore.Open('ReadWrite')
        $rootStore.Add($cert)
        $rootStore.Close()
    }

    # --- Install the package. ---
    Write-Host "Installing opendisplay-idd from '$infPath'..."
    & pnputil.exe /add-driver "$infPath" /install
    if ($LASTEXITCODE -ne 0) {
        throw "pnputil /add-driver failed with exit code $LASTEXITCODE"
    }

    Write-Host 'opendisplay-idd installed.'
}

function Uninstall-DriverPackage {
    Write-Host 'Locating the installed opendisplay-idd driver package...'
    $published = & pnputil.exe /enum-drivers |
        Select-String -Pattern 'Published Name|Original Name' -Context 0, 0 |
        Out-String

    $oemInfName = $null
    $blocks = $published -split "Published Name\s*:\s*"
    foreach ($block in $blocks) {
        if ($block -match 'opendisplay-idd\.inf') {
            $oemInfName = ($block -split "`r?`n")[0].Trim()
            break
        }
    }

    if (-not $oemInfName) {
        Write-Host 'opendisplay-idd is not currently installed; nothing to do.'
        return
    }

    Write-Host "Removing published driver package '$oemInfName'..."
    & pnputil.exe /delete-driver $oemInfName /uninstall /force
    if ($LASTEXITCODE -ne 0) {
        throw "pnputil /delete-driver failed with exit code $LASTEXITCODE"
    }

    Write-Host 'opendisplay-idd uninstalled; no orphaned device should remain.'
}

if ($Uninstall) {
    Uninstall-DriverPackage
} else {
    Install-DriverPackage
}
