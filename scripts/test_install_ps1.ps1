$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'install.ps1') -Binary -Version '0.1.0-beta.1' -Check | Out-Null

$tempRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$fixture = [System.IO.Path]::GetFullPath(
    (Join-Path $tempRoot ('noter-retry-test-' + [System.Guid]::NewGuid().ToString('N')))
)
if (-not $fixture.StartsWith($tempRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'The installer fixture is outside the temporary directory.'
}

New-Item -ItemType Directory -Path $fixture | Out-Null
try {
    $binDir = Join-Path $fixture 'bin'
    New-Item -ItemType Directory -Path $binDir | Out-Null
    $installed = Join-Path $binDir 'noter.exe'
    $previous = Join-Path $binDir 'noter.exe.old'
    Set-Content -LiteralPath $previous -Value 'last working binary' -NoNewline

    $downloadReached = $false
    function Invoke-WebRequest {
        param([string]$Uri, [string]$OutFile, [switch]$UseBasicParsing)

        $script:downloadReached = $true
        $downloadDir = [System.IO.Path]::GetFullPath((Split-Path -Parent $OutFile))
        $expectedPrefix = $script:tempRoot.TrimEnd('\') + '\'
        if (-not $downloadDir.StartsWith($expectedPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw 'The mocked download is outside the temporary directory.'
        }
        throw 'Fixture download stopped.'
    }
    try {
        Install-FromRelease -RequestedVersion '0.1.0-beta.1' -ResolvedInstallRoot $fixture
        throw 'The mocked download unexpectedly succeeded.'
    } catch {
        if ($_.Exception.Message -ne 'Fixture download stopped.') {
            throw
        }
    }
    if (-not $downloadReached) {
        throw 'The installer did not reach the mocked download.'
    }

    if ((Get-Content -LiteralPath $installed -Raw) -ne 'last working binary') {
        throw 'An interrupted install did not restore the last working binary.'
    }
    if (Test-Path -LiteralPath $previous) {
        throw 'The restored backup still occupies its old name.'
    }

    Set-Content -LiteralPath $previous -Value 'older backup' -NoNewline
    Restore-InterruptedBinaryInstall -BinDir $binDir
    if ((Get-Content -LiteralPath $installed -Raw) -ne 'last working binary' -or
        (Get-Content -LiteralPath $previous -Raw) -ne 'older backup') {
        throw 'An intact install or its backup changed during retry recovery.'
    }

    Remove-Item -LiteralPath $installed, $previous
    Set-Content -LiteralPath $previous -Value 'last working binary' -NoNewline
    function Get-NewestReleaseTag {
        throw 'Fixture release lookup stopped.'
    }
    try {
        Install-FromRelease -RequestedVersion 'latest' -ResolvedInstallRoot $fixture
        throw 'The mocked release lookup unexpectedly succeeded.'
    } catch {
        if ($_.Exception.Message -ne 'Fixture release lookup stopped.') {
            throw
        }
    }
    if ((Get-Content -LiteralPath $installed -Raw) -ne 'last working binary' -or
        (Test-Path -LiteralPath $previous)) {
        throw 'A failed release lookup did not restore the last working binary.'
    }

    Remove-Item -LiteralPath $installed
    Set-Content -LiteralPath $previous -Value 'last working binary' -NoNewline
    try {
        Install-FromSource -ResolvedSource (Join-Path $fixture 'missing-source') -ResolvedInstallRoot $fixture
        throw 'The missing source unexpectedly passed validation.'
    } catch {
        if (-not $_.Exception.Message.Contains('Noter source manifest not found')) {
            throw
        }
    }
    if ((Get-Content -LiteralPath $installed -Raw) -ne 'last working binary' -or
        (Test-Path -LiteralPath $previous)) {
        throw 'A failed source preflight did not restore the last working binary.'
    }

    Remove-Item -LiteralPath $installed
    Set-Content -LiteralPath $previous -Value 'last working binary' -NoNewline
    function Get-UserPathEntries { @() }
    Uninstall-Noter -ResolvedInstallRoot $fixture | Out-Null
    if ((Test-Path -LiteralPath $installed) -or (Test-Path -LiteralPath $previous)) {
        throw 'Uninstall left a retained binary that a future retry could restore.'
    }

    New-Item -ItemType Directory -Path $previous | Out-Null
    try {
        Restore-InterruptedBinaryInstall -BinDir $binDir
        throw 'A directory at the backup name was accepted.'
    } catch {
        if ($_.Exception.Message -ne 'An interrupted install backup is not an ordinary file.') {
            throw
        }
    }
    if (-not (Test-Path -LiteralPath $previous -PathType Container)) {
        throw 'The invalid backup was removed.'
    }
} finally {
    if ($fixture.StartsWith($tempRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}

Write-Output 'Windows interrupted-install recovery tests passed.'
