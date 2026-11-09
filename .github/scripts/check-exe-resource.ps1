# Checks the icon and version resource of rick.exe (issue 88,
# specs/app-icons.md), as the release workflow's smoke test does on
# Windows: the version information Explorer's Properties dialog shows —
# ProductName "Rick", FileDescription "Rick file manager", FileVersion and
# ProductVersion starting with the release's version, a LegalCopyright
# naming the Apache License — and an icon the shell extracts from the
# executable.
#
#   pwsh -File check-exe-resource.ps1 <rick.exe> <version>
#       <version> is the release's version; its part before any
#       pre-release suffix must open FileVersion and ProductVersion.
param(
    [Parameter(Mandatory = $true)] [string] $Exe,
    [Parameter(Mandatory = $true)] [string] $Version
)
$ErrorActionPreference = "Stop"
$release = $Version.Split("-")[0]
$info = (Get-Item $Exe).VersionInfo
$info | Format-List ProductName, FileDescription, FileVersion, ProductVersion, LegalCopyright, OriginalFilename
if ($info.ProductName -ne "Rick") { throw "ProductName is '$($info.ProductName)', not Rick" }
if ($info.FileDescription -ne "Rick file manager") { throw "FileDescription is '$($info.FileDescription)'" }
if (-not $info.FileVersion.StartsWith($release)) { throw "FileVersion '$($info.FileVersion)' does not start with $release" }
if (-not $info.ProductVersion.StartsWith($release)) { throw "ProductVersion '$($info.ProductVersion)' does not start with $release" }
if ($info.LegalCopyright -notmatch "Apache License") { throw "LegalCopyright '$($info.LegalCopyright)' does not name the Apache License" }
Add-Type -AssemblyName System.Drawing
$icon = [System.Drawing.Icon]::ExtractAssociatedIcon((Resolve-Path $Exe).Path)
if ($null -eq $icon) { throw "no icon in $Exe" }
"icon: $($icon.Width)x$($icon.Height)"
"the icon and version resource of $Exe are in place"
