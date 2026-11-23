#Requires -Version 7.4
[CmdletBinding(DefaultParameterSetName='System')]
param(
    [Parameter(Mandatory,ParameterSetName='System')][string] $Remote,
    [Parameter(Mandatory,ParameterSetName='System')][string] $Selection,
    [Parameter(Mandatory,ParameterSetName='System')][string] $OutputParent,
    [Parameter(Mandatory,ParameterSetName='Component')][string] $Component,
    [Parameter(Mandatory,ParameterSetName='Component')][ValidateSet('build','test')][string] $CargoOperation,
    [switch] $Offline
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
if ($env:VSCMD_ARG_TGT_ARCH -ne 'x64') {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $vs = & $vswhere -latest -products '*' -version '[17.0,18.0)' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (!$vs) { throw 'Install the x64 C++ toolchain and Windows SDK for Visual Studio 2022.' }
    & (Join-Path $vs 'Common7/Tools/Launch-VsDevShell.ps1') -Arch amd64 -HostArch amd64 -SkipAutomaticLocation
}
if ($PSCmdlet.ParameterSetName -eq 'System') {
    Push-Location -LiteralPath $Remote
    try {
        & (Join-Path $Remote 'scripts/build-native-windows.ps1') -Selection $Selection -OutputParent $OutputParent -Offline:$Offline
    } finally {
        Pop-Location
    }
} else {
    Push-Location -LiteralPath $Component
    try {
        if ($Offline) { $env:CARGO_NET_OFFLINE = 'true' }
        cargo.exe $CargoOperation --locked --release
    } finally {
        Pop-Location
    }
}
