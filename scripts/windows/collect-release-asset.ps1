<#
.SYNOPSIS
Rename the Tauri NSIS installer and the CLI into stable release asset names.

.DESCRIPTION
The Windows counterpart of scripts/package-release.sh. Tauri names the
installer after the product version; the release job needs a name derived from
the tag so SHA256SUMS.txt and the download links stay predictable.
#>
param(
    [Parameter(Mandatory = $true)]
    [string]$BundleDirectory,

    [Parameter(Mandatory = $true)]
    [string]$ReleaseDirectory,

    [Parameter(Mandatory = $true)]
    [string]$Version,

    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'

if ($Version -notmatch '^\d+\.\d+\.\d+') {
    throw "Version must start with MAJOR.MINOR.PATCH: $Version"
}

$bundle = (Resolve-Path -LiteralPath (Join-Path $BundleDirectory 'nsis')).Path
$preferredInstaller = Join-Path $bundle "Lumen Cut_${Version}_x64-setup.exe"
if (Test-Path -LiteralPath $preferredInstaller -PathType Leaf) {
    $installer = Get-Item -LiteralPath $preferredInstaller
}
else {
    $installers = @(Get-ChildItem -LiteralPath $bundle -File -Filter '*-setup.exe')
    if ($installers.Count -ne 1) {
        throw "Expected $preferredInstaller or exactly one NSIS installer in $bundle, found $($installers.Count)"
    }
    $installer = $installers[0]
}

$cli = Join-Path (Resolve-Path -LiteralPath $ReleaseDirectory).Path 'lumen-cut-cli.exe'
if (-not (Test-Path -LiteralPath $cli -PathType Leaf)) {
    throw "Missing release artifact: $cli"
}

if (Test-Path -LiteralPath $OutputDirectory) {
    Get-ChildItem -LiteralPath $OutputDirectory -Force | Remove-Item -Recurse -Force
}
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null

$installerAsset = Join-Path $OutputDirectory "lumen-cut_${Version}_x64-setup.exe"
Copy-Item -LiteralPath $installer.FullName -Destination $installerAsset -Force

# The CLI ships zipped so browsers do not flag a bare .exe download.
$cliAsset = Join-Path $OutputDirectory "lumen-cut-cli_${Version}_x86_64-pc-windows-msvc.zip"
Compress-Archive -LiteralPath $cli -DestinationPath $cliAsset -Force

Push-Location $OutputDirectory
try {
    Get-ChildItem -File -Filter 'lumen-cut*' |
        Get-FileHash -Algorithm SHA256 |
        ForEach-Object { "{0}  {1}" -f $_.Hash.ToLower(), (Split-Path $_.Path -Leaf) } |
        Set-Content -Path 'SHA256SUMS.txt' -Encoding ascii
}
finally {
    Pop-Location
}

Write-Output "Release artifacts: $OutputDirectory"
Get-ChildItem -LiteralPath $OutputDirectory -File | Sort-Object Name | ForEach-Object { $_.FullName }
