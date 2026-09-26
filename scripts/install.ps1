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
    [switch]$Binary,

    [Parameter()]
    [switch]$Uninstall,

    [Parameter()]
    [switch]$Check
)

# Installs Noter on Windows.
#
# From a checkout, this builds and installs the locked source with Cargo.
# Elsewhere, or with -Binary, it downloads a release archive, verifies its
# SHA-256 sidecar, and installs the binary. Everything runs from
# Install-Noter, so a download of this script that stops partway executes
# nothing.

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$NoterRepository = 'blisspixel/noter'

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

# Returns the newest release tag, prereleases included. GitHub's
# releases/latest skips prereleases, so it cannot find one here.
function Get-NewestReleaseTag {
    try {
        $releases = Invoke-RestMethod -Uri "https://api.github.com/repos/$NoterRepository/releases?per_page=1" -UseBasicParsing
    } catch {
        throw "Could not ask GitHub for the newest release ($($_.Exception.Message)). GitHub limits anonymous requests; wait, or pass -Version 0.1.0-beta.1."
    }
    $newest = @($releases) | Select-Object -First 1
    if ($null -eq $newest -or [string]::IsNullOrWhiteSpace($newest.tag_name)) {
        throw 'Could not find a published Noter release.'
    }
    $newest.tag_name
}

function Get-ReleaseTag {
    param([Parameter(Mandatory)][string]$Requested)

    if ($Requested -eq 'latest') {
        return Get-NewestReleaseTag
    }
    if ($Requested.StartsWith('v')) { $Requested } else { "v$Requested" }
}

# Reads and writes the user PATH through the registry so %VARIABLE% entries
# stay unexpanded and the value keeps its REG_EXPAND_SZ type.
function Get-UserPathEntries {
    $key = Get-Item -LiteralPath 'HKCU:\Environment'
    $raw = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    @($raw -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
}

function Set-UserPathEntries {
    param([Parameter(Mandatory)][AllowEmptyCollection()][string[]]$Entries)

    Set-ItemProperty -LiteralPath 'HKCU:\Environment' -Name 'Path' -Value ($Entries -join ';') -Type ExpandString
    # Tell running programs such as Explorer that the environment changed, so
    # terminals opened from them see the new PATH.
    if (-not ('Noter.EnvironmentBroadcast' -as [type])) {
        Add-Type -Namespace 'Noter' -Name 'EnvironmentBroadcast' -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true, CharSet = System.Runtime.InteropServices.CharSet.Unicode)]
public static extern System.IntPtr SendMessageTimeout(System.IntPtr hWnd, uint msg, System.UIntPtr wParam, string lParam, uint flags, uint timeout, out System.UIntPtr result);
'@
    }
    $result = [System.UIntPtr]::Zero
    [void][Noter.EnvironmentBroadcast]::SendMessageTimeout([System.IntPtr]0xffff, 0x1A, [System.UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$result)
}

function Test-SamePath {
    param([string]$Left, [string]$Right)
    [string]::Equals($Left.TrimEnd('\'), $Right.TrimEnd('\'), [System.StringComparison]::OrdinalIgnoreCase)
}

function Install-FromSource {
    param(
        [Parameter(Mandatory)][string]$ResolvedSource,
        [Parameter(Mandatory)][string]$ResolvedInstallRoot,
        [switch]$CheckOnly
    )

    $manifest = Join-Path $ResolvedSource 'Cargo.toml'
    if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) {
        throw "Noter source manifest not found at '$manifest'."
    }

    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if ($null -eq $cargo) {
        throw 'Cargo is required. Install the Rust toolchain from https://rustup.rs, then retry.'
    }

    $metadataExitCode = 0
    Push-Location -LiteralPath $ResolvedSource
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
        throw "The workspace at '$ResolvedSource' does not contain the Noter package."
    }

    if ($CheckOnly) {
        Write-Output "Validated Noter $($noterPackage.version) at '$ResolvedSource'."
        return
    }

    $arguments = @(
        'install', '--path', $ResolvedSource, '--locked', '--force',
        '--root', $ResolvedInstallRoot
    )
    $installExitCode = 0
    Push-Location -LiteralPath $ResolvedSource
    try {
        & $cargo.Source @arguments
        $installExitCode = $LASTEXITCODE
    } finally {
        Pop-Location
    }
    if ($installExitCode -ne 0) {
        throw "Cargo failed to install Noter with exit code $installExitCode."
    }

    $installedBinary = Join-Path $ResolvedInstallRoot 'bin\noter.exe'
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
}

function Install-FromRelease {
    param(
        [Parameter(Mandatory)][string]$RequestedVersion,
        [Parameter(Mandatory)][string]$ResolvedInstallRoot,
        [switch]$CheckOnly
    )

    # Only an x64 build is published. Windows on ARM runs it through x64
    # emulation.
    $target = 'x86_64-pc-windows-msvc'
    $tag = Get-ReleaseTag -Requested $RequestedVersion
    $archiveName = "noter-$target.zip"
    $archiveUrl = "https://github.com/$NoterRepository/releases/download/$tag/$archiveName"

    if ($CheckOnly) {
        Write-Output "Validated release $tag for $target from $archiveUrl."
        return
    }

    $binDir = Join-Path $ResolvedInstallRoot 'bin'
    New-Item -ItemType Directory -Path $binDir -Force | Out-Null
    $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ('noter-install-' + [System.Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
    try {
        $tempArchive = Join-Path $tempDir $archiveName
        $tempChecksum = Join-Path $tempDir "$archiveName.sha256"
        Write-Host "Downloading Noter $tag for $target..."
        Invoke-WebRequest -Uri $archiveUrl -OutFile $tempArchive -UseBasicParsing
        Invoke-WebRequest -Uri "$archiveUrl.sha256" -OutFile $tempChecksum -UseBasicParsing

        # The sidecar comes from the same release, so this proves the archive
        # arrived intact, not who built it. INSTALLATION.md shows how to verify
        # the GitHub build attestation as well.
        # The sidecar reads "<hash> *<file>". Split with a pattern: String.Split
        # with a string argument splits on characters in Windows PowerShell 5.1
        # but on the whole string in PowerShell 7.
        $expectedHash = ((Get-Content -LiteralPath $tempChecksum -Raw).Trim() -split '\s+')[0].ToLowerInvariant()
        $actualHash = (Get-FileHash -LiteralPath $tempArchive -Algorithm SHA256).Hash.ToLowerInvariant()
        if ([string]::IsNullOrEmpty($expectedHash) -or $actualHash -ne $expectedHash) {
            throw "Checksum mismatch for ${archiveName}: expected $expectedHash, got $actualHash."
        }

        $extractDir = Join-Path $tempDir 'extracted'
        Expand-Archive -LiteralPath $tempArchive -DestinationPath $extractDir -Force
        $extractedBinary = Get-ChildItem -LiteralPath $extractDir -Filter 'noter.exe' -Recurse | Select-Object -First 1
        if ($null -eq $extractedBinary) {
            throw 'The release archive did not contain noter.exe.'
        }
        $expectedVersion = "noter $($tag.TrimStart('v'))"
        $stagedVersion = (Invoke-NoterCli -Binary $extractedBinary.FullName -Arguments @('--version')).StdOut.Trim()
        if ($stagedVersion -ne $expectedVersion) {
            throw "The downloaded binary reported '$stagedVersion', expected '$expectedVersion'."
        }

        # Copy the new file beside the destination first, so the final step
        # is a rename on one volume. A running noter.exe cannot be
        # overwritten but can be renamed, so move it aside, rename the new
        # file into place, and restore the old one if that fails.
        $installedBinary = Join-Path $binDir 'noter.exe'
        $stagedBinary = Join-Path $binDir 'noter.exe.new'
        $previousBinary = Join-Path $binDir 'noter.exe.old'
        Copy-Item -LiteralPath $extractedBinary.FullName -Destination $stagedBinary -Force
        Remove-Item -LiteralPath $previousBinary -Force -ErrorAction SilentlyContinue
        $hadPrevious = Test-Path -LiteralPath $installedBinary
        if ($hadPrevious) {
            Move-Item -LiteralPath $installedBinary -Destination $previousBinary -Force
        }
        try {
            Move-Item -LiteralPath $stagedBinary -Destination $installedBinary -Force
        } catch {
            if ($hadPrevious) {
                Move-Item -LiteralPath $previousBinary -Destination $installedBinary -Force
            }
            Remove-Item -LiteralPath $stagedBinary -Force -ErrorAction SilentlyContinue
            throw
        }
        Remove-Item -LiteralPath $previousBinary -Force -ErrorAction SilentlyContinue

        $entries = @(Get-UserPathEntries)
        if (-not ($entries | Where-Object { Test-SamePath $_ $binDir })) {
            Set-UserPathEntries -Entries (@($entries) + $binDir)
            Write-Output "Added $binDir to your user PATH. Open a new terminal to run noter by name."
        }
        Write-Output "Installed $stagedVersion at '$installedBinary'."
    } finally {
        Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Uninstall-Noter {
    param(
        [Parameter(Mandatory)][string]$ResolvedInstallRoot,
        [switch]$CheckOnly
    )

    $binDir = Join-Path $ResolvedInstallRoot 'bin'
    $installedBinary = Join-Path $binDir 'noter.exe'
    if ($CheckOnly) {
        Write-Output "Would remove '$installedBinary'."
        return
    }
    if (Test-Path -LiteralPath $installedBinary) {
        Remove-Item -LiteralPath $installedBinary -Force
        Write-Output "Removed '$installedBinary'. Documents and settings were not touched."
    } else {
        Write-Output "No Noter binary at '$installedBinary'."
    }
    $entries = @(Get-UserPathEntries)
    $remaining = @($entries | Where-Object { -not (Test-SamePath $_ $binDir) })
    $otherFiles = @(Get-ChildItem -LiteralPath $binDir -ErrorAction SilentlyContinue)
    if ($remaining.Count -ne $entries.Count -and $otherFiles.Count -eq 0) {
        Set-UserPathEntries -Entries $remaining
        Write-Output "Removed $binDir from your user PATH."
    }
}

function Install-Noter {
    param(
        [string]$Source,
        [string]$InstallRoot,
        [string]$Version = 'latest',
        [switch]$FromSource,
        [switch]$Binary,
        [switch]$Uninstall,
        [switch]$Check
    )

    # PowerShell 5.1 defaults to older TLS and draws a slow progress bar for
    # every downloaded block.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $ProgressPreference = 'SilentlyContinue'

    $sourceCandidate = if (-not [string]::IsNullOrWhiteSpace($Source)) {
        (Resolve-Path -LiteralPath $Source).Path
    } elseif (-not [string]::IsNullOrWhiteSpace($PSScriptRoot)) {
        # A file under scripts/ of a checkout. Run through Invoke-Expression,
        # there is no script root and no checkout is assumed.
        $parent = Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..') -ErrorAction SilentlyContinue
        if ($null -ne $parent -and (Select-String -LiteralPath (Join-Path $parent.Path 'Cargo.toml') -Pattern '^name = "noter"$' -Quiet -ErrorAction SilentlyContinue)) {
            $parent.Path
        }
    }
    $useSource = -not $Binary -and ($FromSource -or -not [string]::IsNullOrWhiteSpace($Source) -or $null -ne $sourceCandidate)
    if ($useSource -and $null -eq $sourceCandidate) {
        throw '-FromSource needs a checkout; run the script from one or pass -Source PATH.'
    }
    if ($useSource -and $Version -ne 'latest') {
        throw '-Version selects a release download; add -Binary, or omit -Version to build this checkout.'
    }

    $cargoHome = if ([string]::IsNullOrWhiteSpace($env:CARGO_HOME)) {
        Join-Path $env:USERPROFILE '.cargo'
    } else {
        [System.IO.Path]::GetFullPath($env:CARGO_HOME)
    }
    if (-not [string]::IsNullOrWhiteSpace($env:CARGO_HOME)) {
        $env:CARGO_HOME = $cargoHome
    }
    $resolvedInstallRoot = if (-not [string]::IsNullOrWhiteSpace($InstallRoot)) {
        [System.IO.Path]::GetFullPath($InstallRoot)
    } elseif (-not [string]::IsNullOrWhiteSpace($env:CARGO_INSTALL_ROOT)) {
        [System.IO.Path]::GetFullPath($env:CARGO_INSTALL_ROOT)
    } elseif ($useSource) {
        $cargoHome
    } elseif (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        Join-Path $env:LOCALAPPDATA 'Programs\Noter'
    } else {
        throw 'An installation root could not be determined; pass -InstallRoot.'
    }

    if ($Uninstall) {
        Uninstall-Noter -ResolvedInstallRoot $resolvedInstallRoot -CheckOnly:$Check
    } elseif ($useSource) {
        Install-FromSource -ResolvedSource $sourceCandidate -ResolvedInstallRoot $resolvedInstallRoot -CheckOnly:$Check
    } else {
        Install-FromRelease -RequestedVersion $Version -ResolvedInstallRoot $resolvedInstallRoot -CheckOnly:$Check
    }
}

Install-Noter @PSBoundParameters
