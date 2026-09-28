$ErrorActionPreference = 'Stop'

# Hosted Windows Server images can leave the Cloud Files minifilter unloaded.
# Native tests require it; production recovery still fails closed without it.
$filters = & fltmc.exe filters
if ($LASTEXITCODE -ne 0 -or ($filters -match 'failed with error:')) {
    throw 'Cannot inspect Windows file-system filters for Cloud Files tests.'
}

if (-not ($filters -match '^\s*CldFlt\s')) {
    $loadResult = & fltmc.exe load CldFlt
    if ($LASTEXITCODE -ne 0 -or ($loadResult -match 'failed with error:')) {
        throw 'The Windows Cloud Files filter could not be loaded for native recovery tests.'
    }
}

$filters = & fltmc.exe filters
if (
    $LASTEXITCODE -ne 0 -or
    ($filters -match 'failed with error:') -or
    -not ($filters -match '^\s*CldFlt\s')
) {
    throw 'The Windows Cloud Files filter is unavailable for native recovery tests.'
}

# Hosted images can load CldFlt without attaching it to the NTFS test volumes.
# Attach the disposable runner's workspace and temporary volumes explicitly.
$testPaths = @($env:RUNNER_TEMP, [IO.Path]::GetTempPath(), (Get-Location).Path)
$volumes = $testPaths | Where-Object { $_ } | ForEach-Object {
    [IO.Path]::GetPathRoot($_).TrimEnd('\')
} | Sort-Object -Unique

foreach ($volume in $volumes) {
    $instances = & fltmc.exe instances -v $volume
    if ($LASTEXITCODE -ne 0 -or ($instances -match 'failed with error:')) {
        throw "Cannot inspect Cloud Files filter attachment on $volume."
    }
    if (-not ($instances -match '^\s*CldFlt\s')) {
        $attachResult = & fltmc.exe attach CldFlt $volume
        if ($LASTEXITCODE -ne 0 -or ($attachResult -match 'failed with error:')) {
            throw "The Cloud Files filter could not attach to $volume."
        }
    }
    $instances = & fltmc.exe instances -v $volume
    if (
        $LASTEXITCODE -ne 0 -or
        ($instances -match 'failed with error:') -or
        -not ($instances -match '^\s*CldFlt\s')
    ) {
        throw "The Cloud Files filter is not attached to $volume."
    }
}
