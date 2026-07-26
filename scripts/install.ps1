[CmdletBinding()]
param(
    [Parameter()]
    [string]$Source,

    [Parameter()]
    [string]$InstallRoot,

    [Parameter()]
    [switch]$Check
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$effectiveCargoHome = if ([string]::IsNullOrWhiteSpace($env:CARGO_HOME)) {
    Join-Path $env:USERPROFILE '.cargo'
} else {
    [System.IO.Path]::GetFullPath($env:CARGO_HOME)
}
if (-not [string]::IsNullOrWhiteSpace($env:CARGO_HOME)) {
    $env:CARGO_HOME = $effectiveCargoHome
}

if ([string]::IsNullOrWhiteSpace($Source)) {
    $Source = Join-Path $PSScriptRoot '..'
}
$resolvedSource = (Resolve-Path -LiteralPath $Source).Path
$manifest = Join-Path $resolvedSource 'Cargo.toml'
if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) {
    throw "Noter source manifest not found at '$manifest'."
}

$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if ($null -eq $cargo) {
    throw 'Cargo is required. Install the Rust toolchain from https://rustup.rs, then retry.'
}

$metadataExitCode = 0
Push-Location -LiteralPath $resolvedSource
try {
    $metadata = & $cargo.Source metadata --locked --no-deps --format-version 1 --manifest-path $manifest |
        ConvertFrom-Json
    $metadataExitCode = $LASTEXITCODE
} finally {
    Pop-Location
}
if ($metadataExitCode -ne 0) {
    throw 'Cargo could not validate the locked Noter workspace.'
}
$noterPackage = $metadata.packages | Where-Object { $_.name -eq 'noter' } | Select-Object -First 1
if ($null -eq $noterPackage) {
    throw "The workspace at '$resolvedSource' does not contain the Noter package."
}

$resolvedInstallRoot = if (-not [string]::IsNullOrWhiteSpace($InstallRoot)) {
    [System.IO.Path]::GetFullPath($InstallRoot)
} elseif (-not [string]::IsNullOrWhiteSpace($env:CARGO_INSTALL_ROOT)) {
    [System.IO.Path]::GetFullPath($env:CARGO_INSTALL_ROOT)
} else {
    $effectiveCargoHome
}
$arguments = @(
    'install', '--path', $resolvedSource, '--locked', '--force',
    '--root', $resolvedInstallRoot
)

if ($Check) {
    Write-Output "Validated Noter $($noterPackage.version) at '$resolvedSource'."
    exit 0
}

$installExitCode = 0
Push-Location -LiteralPath $resolvedSource
try {
    & $cargo.Source @arguments
    $installExitCode = $LASTEXITCODE
} finally {
    Pop-Location
}
if ($installExitCode -ne 0) {
    throw "Cargo failed to install Noter with exit code $installExitCode."
}

$installedBinary = Join-Path $resolvedInstallRoot 'bin\noter.exe'
if (-not (Test-Path -LiteralPath $installedBinary -PathType Leaf)) {
    throw "Cargo reported success, but '$installedBinary' was not found."
}
$versionOutput = [System.IO.Path]::GetTempFileName()
$versionError = [System.IO.Path]::GetTempFileName()
try {
    $versionProcess = Start-Process -FilePath $installedBinary -ArgumentList '--version' `
        -Wait -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $versionOutput -RedirectStandardError $versionError
    $installedVersion = (Get-Content -LiteralPath $versionOutput -Raw).Trim()
} finally {
    Remove-Item -LiteralPath $versionOutput, $versionError -Force
}
if ($versionProcess.ExitCode -ne 0 -or $installedVersion -ne "noter $($noterPackage.version)") {
    throw "The installed executable did not report the expected Noter version $($noterPackage.version)."
}
Write-Output "Installed Noter $($noterPackage.version) at '$installedBinary'."
