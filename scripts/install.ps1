[CmdletBinding()]
param(
    [Parameter()]
    [string]$Source,

    [Parameter()]
    [string]$InstallRoot,

    [Parameter()]
    [string]$Version = 'latest',

    [Parameter()]
    [switch]$FromSource,

    [Parameter()]
    [switch]$Check
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Invoke-NoterCli {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [string]$Binary,

        [Parameter(Mandatory)]
        [string[]]$Arguments
    )

    $standardOutput = [System.IO.Path]::GetTempFileName()
    $standardError = [System.IO.Path]::GetTempFileName()
    try {
        $process = Start-Process -FilePath $Binary -ArgumentList $Arguments `
            -Wait -PassThru -WindowStyle Hidden `
            -RedirectStandardOutput $standardOutput -RedirectStandardError $standardError
        [PSCustomObject]@{
            ExitCode = $process.ExitCode
            StdOut = [string](Get-Content -LiteralPath $standardOutput -Raw)
            StdErr = [string](Get-Content -LiteralPath $standardError -Raw)
        }
    } finally {
        Remove-Item -LiteralPath $standardOutput, $standardError -Force
    }
}

$effectiveCargoHome = if ([string]::IsNullOrWhiteSpace($env:CARGO_HOME)) {
    Join-Path $env:USERPROFILE '.cargo'
} else {
    [System.IO.Path]::GetFullPath($env:CARGO_HOME)
}
if (-not [string]::IsNullOrWhiteSpace($env:CARGO_HOME)) {
    $env:CARGO_HOME = $effectiveCargoHome
}

$resolvedInstallRoot = if (-not [string]::IsNullOrWhiteSpace($InstallRoot)) {
    [System.IO.Path]::GetFullPath($InstallRoot)
} elseif (-not [string]::IsNullOrWhiteSpace($env:CARGO_INSTALL_ROOT)) {
    [System.IO.Path]::GetFullPath($env:CARGO_INSTALL_ROOT)
} elseif (Test-Path -LiteralPath (Join-Path $effectiveCargoHome 'bin')) {
    $effectiveCargoHome
} elseif (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
    Join-Path $env:LOCALAPPDATA 'Programs\noter'
} else {
    $effectiveCargoHome
}

$hasExplicitSource = -not [string]::IsNullOrWhiteSpace($Source)
$localSourceCandidate = if ($hasExplicitSource) {
    (Resolve-Path -LiteralPath $Source -ErrorAction SilentlyContinue)
} elseif (-not [string]::IsNullOrWhiteSpace($PSScriptRoot)) {
    (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..') -ErrorAction SilentlyContinue)
} else {
    $null
}
$isSourceAvailable = $null -ne $localSourceCandidate -and (Test-Path -LiteralPath (Join-Path $localSourceCandidate.Path 'Cargo.toml') -PathType Leaf)

if ($FromSource -or ($isSourceAvailable -and $hasExplicitSource)) {
    $resolvedSource = $localSourceCandidate.Path
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
    $versionResult = Invoke-NoterCli -Binary $installedBinary -Arguments @('--version')
    $installedVersion = $versionResult.StdOut.Trim()
    if (
        $versionResult.ExitCode -ne 0 -or
        -not [string]::IsNullOrEmpty($versionResult.StdErr) -or
        $installedVersion -ne "noter $($noterPackage.version)"
    ) {
        throw "The installed executable did not report the expected Noter version $($noterPackage.version)."
    }

    $invalidResult = Invoke-NoterCli -Binary $installedBinary -Arguments @('--theme', 'invalid')
    if (
        $invalidResult.ExitCode -ne 2 -or
        -not [string]::IsNullOrEmpty($invalidResult.StdOut) -or
        -not $invalidResult.StdErr.Contains('unknown theme `invalid`; expected system, light, dark, green, or amber') -or
        -not $invalidResult.StdErr.Contains('Usage:')
    ) {
        throw 'The installed executable did not preserve the release command-line error contract.'
    }
    Write-Output "Installed Noter $($noterPackage.version) at '$installedBinary'."
} else {
    $repo = 'blisspixel/noter'
    $target = 'x86_64-pc-windows-msvc'
    $archiveName = "noter-$target.zip"
    $baseReleaseUrl = if ($Version -eq 'latest') {
        "https://github.com/$repo/releases/latest/download"
    } else {
        "https://github.com/$repo/releases/download/$Version"
    }
    $archiveUrl = "$baseReleaseUrl/$archiveName"
    $checksumUrl = "$archiveUrl.sha256"

    if ($Check) {
        Write-Output "Validated release target $target for Noter ($Version)."
        exit 0
    }

    $binDir = Join-Path $resolvedInstallRoot 'bin'
    if (-not (Test-Path -LiteralPath $binDir)) {
        New-Item -ItemType Directory -Path $binDir -Force | Out-Null
    }

    $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("noter-install-" + [System.Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
    try {
        $tempArchive = Join-Path $tempDir $archiveName
        $tempChecksum = Join-Path $tempDir "$archiveName.sha256"

        Write-Host "Downloading Noter ($Version)..."
        Invoke-WebRequest -Uri $archiveUrl -OutFile $tempArchive -UseBasicParsing
        Invoke-WebRequest -Uri $checksumUrl -OutFile $tempChecksum -UseBasicParsing

        $expectedHash = (Get-Content -LiteralPath $tempChecksum -Raw).Trim().Split(" `t`r`n")[0].ToLowerInvariant()
        $actualHash = (Get-FileHash -LiteralPath $tempArchive -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actualHash -ne $expectedHash) {
            throw "Checksum verification failed! Expected $expectedHash, got $actualHash."
        }

        Expand-Archive -LiteralPath $tempArchive -DestinationPath $tempDir -Force
        $extractedBinary = Get-ChildItem -Path $tempDir -Filter 'noter.exe' -Recurse | Select-Object -First 1
        if ($null -eq $extractedBinary) {
            throw "Archive did not contain noter.exe."
        }

        $installedBinary = Join-Path $binDir 'noter.exe'
        Copy-Item -LiteralPath $extractedBinary.FullName -Destination $installedBinary -Force

        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if ($null -eq $userPath) { $userPath = '' }
        $pathEntries = $userPath -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
        if ($pathEntries -notcontains $binDir) {
            [Environment]::SetEnvironmentVariable('Path', "$userPath;$binDir", 'User')
            $env:Path = "$env:Path;$binDir"
        }

        $versionResult = Invoke-NoterCli -Binary $installedBinary -Arguments @('--version')
        if ($versionResult.ExitCode -ne 0) {
            throw "The installed binary failed self-test."
        }
        Write-Output "Installed $($versionResult.StdOut.Trim()) at '$installedBinary'."
    } finally {
        Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
