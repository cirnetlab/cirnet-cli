#Requires -Version 7.4
[CmdletBinding(DefaultParameterSetName='System')]
param(
    [Parameter(Mandatory,ParameterSetName='System')][string] $Remote,
    [Parameter(Mandatory,ParameterSetName='System')][string] $Selection,
    [Parameter(Mandatory,ParameterSetName='System')][string] $OutputParent,
    [Parameter(Mandatory,ParameterSetName='Component')][string] $Component,
    [Parameter(Mandatory,ParameterSetName='Component')][ValidateSet('build','test')][string] $CargoOperation,
    [Parameter(ParameterSetName='Component')][string] $ArtifactAllocator,
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
    $previousTarget = $env:CARGO_TARGET_DIR
    $previousOffline = $env:CARGO_NET_OFFLINE
    $previousTemp = @($env:TEMP, $env:TMP, $env:TMPDIR)
    try {
        if ($null -ne $env:CIRNET_LOCAL_STORAGE_ROOT -or !$env:CARGO_TARGET_DIR) {
            if (!$ArtifactAllocator) {
                $workspace = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
                $ArtifactAllocator = Join-Path $workspace 'cirnet-contracts/tools/artifact-paths/cirnet_artifact_paths/artifact-paths.mjs'
            }
            if ($env:CARGO_BUILD_TARGET_DIR -or $env:CARGO_BUILD_BUILD_DIR -or $env:CARGO_BUILD_DIR) {
                throw 'Use only CARGO_TARGET_DIR for an explicit component build directory.'
            }
            if ($env:CARGO_TARGET_DIR) {
                if ($null -ne $env:CIRNET_LOCAL_STORAGE_ROOT) {
                    $owned = (node $ArtifactAllocator --session-info | ConvertFrom-Json).directory
                    $relative = [IO.Path]::GetRelativePath($owned, $env:CARGO_TARGET_DIR)
                    if ([IO.Path]::IsPathRooted($relative) -or $relative -match '^\.\.(?:[\\/]|$)') {
                        throw 'CARGO_TARGET_DIR conflicts with the inherited artifact session.'
                    }
                }
                $env:CARGO_TARGET_DIR = node $ArtifactAllocator --validate $env:CARGO_TARGET_DIR
            }
            else { $env:CARGO_TARGET_DIR = node $ArtifactAllocator cargo }
        }
        if ($null -ne $env:CIRNET_LOCAL_STORAGE_ROOT) {
            $temp = node $ArtifactAllocator --validate (Join-Path $env:CARGO_TARGET_DIR 'tmp')
            $null = New-Item -ItemType Directory -Path $temp -Force
            $env:TEMP = $env:TMP = $env:TMPDIR = $temp
        }
        if ($Offline) { $env:CARGO_NET_OFFLINE = 'true' }
        cargo.exe $CargoOperation --locked --release
    } finally {
        $env:CARGO_TARGET_DIR = $previousTarget
        $env:CARGO_NET_OFFLINE = $previousOffline
        $env:TEMP, $env:TMP, $env:TMPDIR = $previousTemp
        Pop-Location
    }
}
