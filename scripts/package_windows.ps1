[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if (-not [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
    [System.Runtime.InteropServices.OSPlatform]::Windows
)) {
    throw "This packaging script must run on Windows so the artifact uses the native Windows ABI."
}

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir
Set-Location $ProjectDir

$Manifest = Get-Content (Join-Path $ProjectDir "app/Cargo.toml") -Raw
$PackageSection = [regex]::Match($Manifest, '(?ms)^\[package\]\s*$.*?(?=^\[|\z)')
$VersionMatch = [regex]::Match($PackageSection.Value, '(?m)^version\s*=\s*"([^"]+)"\s*$')
if (-not $VersionMatch.Success) {
    throw "Could not read readwise-app version from app/Cargo.toml"
}
$AppVersion = $VersionMatch.Groups[1].Value

$Architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
$BundleName = "ReadWise-windows-$Architecture-$AppVersion"
$DistDir = Join-Path $ProjectDir "target/dist"
$BundleDir = Join-Path $DistDir $BundleName
$Archive = Join-Path $DistDir "$BundleName.zip"
$BuiltBinary = Join-Path $ProjectDir "target/release/readwise-app.exe"

& cargo makepad desktop build -p readwise-app --release --locked --offline
if ($LASTEXITCODE -ne 0) {
    throw "cargo makepad desktop build failed with exit code $LASTEXITCODE"
}
if (-not (Test-Path $BuiltBinary -PathType Leaf)) {
    throw "Expected release binary was not generated at $BuiltBinary"
}

New-Item -ItemType Directory -Force -Path $DistDir | Out-Null
if (Test-Path $BundleDir) {
    Remove-Item -Recurse -Force $BundleDir
}
if (Test-Path $Archive) {
    Remove-Item -Force $Archive
}
New-Item -ItemType Directory -Path $BundleDir | Out-Null

$DistBinary = Join-Path $BundleDir "ReadWise.exe"
Copy-Item $BuiltBinary $DistBinary
$Notices = Join-Path $BundleDir "third-party/typst-assets"
New-Item -ItemType Directory -Force -Path $Notices | Out-Null
Copy-Item "app/resources/third-party/typst-assets/LICENSE", "app/resources/third-party/typst-assets/NOTICE" $Notices

$SigningThumbprint = $env:READWISE_WINDOWS_CERT_THUMBPRINT
if (-not [string]::IsNullOrWhiteSpace($SigningThumbprint)) {
    $SignTool = Get-Command "signtool.exe" -ErrorAction SilentlyContinue
    if ($null -eq $SignTool) {
        throw "READWISE_WINDOWS_CERT_THUMBPRINT is set, but signtool.exe was not found. Install the Windows SDK."
    }

    $SignArgs = @("sign", "/sha1", $SigningThumbprint, "/fd", "SHA256")
    $TimestampUrl = $env:READWISE_WINDOWS_TIMESTAMP_URL
    if (-not [string]::IsNullOrWhiteSpace($TimestampUrl)) {
        $SignArgs += @("/tr", $TimestampUrl, "/td", "SHA256")
    }
    $SignArgs += $DistBinary

    & $SignTool.Source @SignArgs
    if ($LASTEXITCODE -ne 0) {
        throw "signtool failed with exit code $LASTEXITCODE"
    }

    $Signature = Get-AuthenticodeSignature $DistBinary
    if ($Signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
        throw "Authenticode verification failed: $($Signature.Status) $($Signature.StatusMessage)"
    }
}

@"
ReadWise for Windows
====================

Run ReadWise.exe directly. The application stores its database and imported
books in the current user's platform application-data directory.

If Windows SmartScreen warns about an unsigned build, verify the archive came
from a trusted source. Public releases should be produced with
READWISE_WINDOWS_CERT_THUMBPRINT set to a trusted code-signing certificate.
"@ | Set-Content -Path (Join-Path $BundleDir "README.txt") -Encoding UTF8

Compress-Archive -Path $BundleDir -DestinationPath $Archive -CompressionLevel Optimal
if (-not (Test-Path $Archive -PathType Leaf)) {
    throw "Distribution archive was not generated at $Archive"
}

Write-Host "Windows bundle: $BundleDir"
Write-Host "Distribution archive: $Archive"
if ([string]::IsNullOrWhiteSpace($SigningThumbprint)) {
    Write-Warning "The executable is unsigned. Set READWISE_WINDOWS_CERT_THUMBPRINT for public distribution."
} else {
    Write-Host "The executable is Authenticode signed and verified."
}
