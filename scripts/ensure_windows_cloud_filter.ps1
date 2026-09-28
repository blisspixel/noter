$ErrorActionPreference = 'Stop'

# Hosted Windows Server images can leave the Cloud Files minifilter unloaded.
# Native tests require it; production recovery still fails closed without it.
$filters = & fltmc.exe filters
if ($LASTEXITCODE -ne 0) {
    throw 'Cannot inspect Windows file-system filters for Cloud Files tests.'
}

if (-not ($filters -match '^\s*CldFlt\s')) {
    & fltmc.exe load CldFlt
    if ($LASTEXITCODE -ne 0) {
        throw 'The Windows Cloud Files filter could not be loaded for native recovery tests.'
    }
}

$filters = & fltmc.exe filters
if ($LASTEXITCODE -ne 0 -or -not ($filters -match '^\s*CldFlt\s')) {
    throw 'The Windows Cloud Files filter is unavailable for native recovery tests.'
}
