$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'install.ps1') -Binary -Version '0.1.0-beta.1' -Check | Out-Null

$tempRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$fixture = [System.IO.Path]::GetFullPath(
    (Join-Path $tempRoot ('noter-source-test-' + [System.Guid]::NewGuid().ToString('N')))
)
if (-not $fixture.StartsWith($tempRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'The source installer fixture is outside the temporary directory.'
}

New-Item -ItemType Directory -Path $fixture | Out-Null
try {
    $source = Join-Path $fixture 'source'
    $binDir = Join-Path $fixture 'install\bin'
    New-Item -ItemType Directory -Path $source, $binDir | Out-Null
    Set-Content -LiteralPath (Join-Path $source 'Cargo.toml') -Value 'name = "noter"'
    $installed = Join-Path $binDir 'noter.exe'
    Set-Content -LiteralPath $installed -Value 'last working binary' -NoNewline
    $legacyStage = Join-Path $binDir 'noter.exe.new'
    $victim = Join-Path $fixture 'unrelated.txt'
    Set-Content -LiteralPath $victim -Value 'unrelated content' -NoNewline
    try {
        New-Item -ItemType SymbolicLink -Path $legacyStage -Target $victim -ErrorAction Stop | Out-Null
    } catch {
        Set-Content -LiteralPath $legacyStage -Value 'unrelated stage' -NoNewline
    }

    $script:fakeCargo = Join-Path $fixture 'fake-cargo.ps1'
    $env:TEST_CARGO_ROOT_LOG = Join-Path $fixture 'cargo-root.txt'
    Set-Content -LiteralPath $script:fakeCargo -Value @'
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$CargoArguments)
if ($CargoArguments[0] -eq 'metadata') {
    $global:LASTEXITCODE = 0
    '{"packages":[{"name":"noter","version":"0.1.0-beta.1"}]}'
    return
}
if ($CargoArguments[0] -ne 'install') { throw 'Unexpected Cargo command.' }
$rootIndex = [Array]::IndexOf($CargoArguments, '--root')
if ($rootIndex -lt 0) { throw 'The Cargo install had no staging root.' }
$stageRoot = $CargoArguments[$rootIndex + 1]
Set-Content -LiteralPath $env:TEST_CARGO_ROOT_LOG -Value $stageRoot -NoNewline
$stageBin = Join-Path $stageRoot 'bin'
New-Item -ItemType Directory -Path $stageBin | Out-Null
Set-Content -LiteralPath (Join-Path $stageBin 'noter.exe') -Value 'new verified binary' -NoNewline
$global:LASTEXITCODE = 0
'@

    function Get-Command {
        [CmdletBinding()]
        param([string]$Name)
        if ($Name -ne 'cargo') { throw 'Unexpected command lookup.' }
        [PSCustomObject]@{ Source = $script:fakeCargo }
    }
    $script:reportedVersion = 'noter wrong-version'
    function Invoke-NoterCli {
        param([string]$Binary, [string[]]$Arguments)
        if ($Arguments[0] -eq '--version') {
            return [PSCustomObject]@{ ExitCode = 0; StdOut = $script:reportedVersion; StdErr = '' }
        }
        return [PSCustomObject]@{
            ExitCode = 2
            StdOut = ''
            StdErr = 'unknown theme `invalid`; expected system, light, dark, green, or amber. Usage:'
        }
    }

    try {
        Install-FromSource -ResolvedSource $source -ResolvedInstallRoot (Join-Path $fixture 'install')
        throw 'An invalid staged version unexpectedly installed.'
    } catch {
        if (-not $_.Exception.Message.Contains('staged executable did not report')) { throw }
    }
    if ((Get-Content -LiteralPath $installed -Raw) -ne 'last working binary') {
        throw 'A failed source verification replaced the working executable.'
    }
    $stageRoot = Get-Content -LiteralPath $env:TEST_CARGO_ROOT_LOG -Raw
    $expectedPrefix = $tempRoot.TrimEnd('\') + '\'
    if (-not $stageRoot.StartsWith($expectedPrefix, [System.StringComparison]::OrdinalIgnoreCase) -or
        (Test-Path -LiteralPath $stageRoot)) {
        throw 'The failed source build root was not safely cleaned.'
    }

    $script:reportedVersion = 'noter 0.1.0-beta.1'
    Install-FromSource -ResolvedSource $source -ResolvedInstallRoot (Join-Path $fixture 'install') | Out-Null
    if ((Get-Content -LiteralPath $installed -Raw) -ne 'new verified binary') {
        throw 'A verified source binary did not replace the old executable.'
    }
    if (Test-Path -LiteralPath (Join-Path $binDir 'noter.exe.old')) {
        throw 'A successful source install left its predecessor backup.'
    }
    if (-not (Test-Path -LiteralPath $legacyStage) -or
        (Get-Content -LiteralPath $victim -Raw) -ne 'unrelated content') {
        throw 'The source install used the predictable old stage or changed an unrelated file.'
    }
    if (@(Get-ChildItem -LiteralPath $binDir | Where-Object {
        $_.Name -match '^noter\.exe\.new\.[0-9a-f]{32}$'
    }).Count -ne 0) {
        throw 'The source install left a temporary stage.'
    }
    $stageRoot = Get-Content -LiteralPath $env:TEST_CARGO_ROOT_LOG -Raw
    if (-not $stageRoot.StartsWith($expectedPrefix, [System.StringComparison]::OrdinalIgnoreCase) -or
        (Test-Path -LiteralPath $stageRoot)) {
        throw 'The successful source build root was not safely cleaned.'
    }
} finally {
    Remove-Item Env:TEST_CARGO_ROOT_LOG -ErrorAction SilentlyContinue
    if ($fixture.StartsWith($tempRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}

Write-Output 'Windows staged source-install tests passed.'
