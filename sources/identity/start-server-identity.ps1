#!/usr/bin/env pwsh
param(
  [switch]$FastBuild,
  [switch]$Tidy,
  [switch]$SkipBuild,
  [switch]$SkipBuildWebJs,
  [switch]$SkipBuildTempl,
  [switch]$SkipBuildGoGenerate,
  [switch]$SkipBuildGoModTidy,
  [switch]$SkipBuildGoGet,
  [switch]$SkipBuildGoBuild,
  [switch]$SkipBuildGoExperiment,
  [switch]$Update,
  [switch]$ForceInstallTempl
)

Set-StrictMode -Version Latest

$PSNativeCommandUseErrorActionPreference = $true

if ($PSNativeCommandUseErrorActionPreference) {
  # always true, this is a linter workaround
  $ErrorActionPreference = "Stop"
  $PSDefaultParameterValues['*:ErrorAction'] = 'Stop'
}

$originalVerbosePreference = $VerbosePreference
$VerbosePreference = 'Continue'

Write-Verbose "script: $($MyInvocation.MyCommand.Name)"
Write-Verbose "psscriptroot: $PSScriptRoot"
Write-Verbose "full script path: $PSScriptRoot$([IO.Path]::DirectorySeparatorChar)$($MyInvocation.MyCommand.Name)"
Write-Verbose "originalVerbosePreference: $originalVerbosePreference"
Write-Verbose "VerbosePreference: $VerbosePreference"

if ($FastBuild) {
  $SkipBuildWebJs = $true
  $SkipBuildTempl = $true
  $SkipBuildGoGenerate = $true
  $SkipBuildGoModTidy = $true
  $SkipBuildGoGet = $true
  $SkipBuildGoExperiment = $true
}

if ($Tidy) {
  $SkipBuildGoModTidy = $false
}

try {

  $cwd = Get-Location

  Write-Verbose "Current directory: $cwd"

  try {

    Write-Verbose "Set-Location $PSScriptRoot"

    Set-Location $PSScriptRoot

    if (-not $SkipBuild) {
      Write-Verbose "Building libsql"
      ."$PSScriptRoot/build-libsql.ps1" -ForceInstallTempl:$ForceInstallTempl -Update:$Update -SkipBuildWebJs:$SkipBuildWebJs -SkipBuildTempl:$SkipBuildTempl -SkipBuildGoGenerate:$SkipBuildGoGenerate -SkipBuildGoModTidy:$SkipBuildGoModTidy -SkipBuildGoGet:$SkipBuildGoGet -SkipBuildGoBuild:$SkipBuildGoBuild -SkipBuildGoExperiment:$SkipBuildGoExperiment
    }
    else {
      Write-Verbose "Skipping libsql build"
    }
    $env:CHARM_SERVER_DB_DRIVER = "libsql"

    if ([string]::IsNullOrEmpty($env:TURSO_HOST)) {
      throw "TURSO_HOST environment variable must be set"
    }
    if ([string]::IsNullOrEmpty($env:TURSO_AUTH_TOKEN)) {
      throw "TURSO_AUTH_TOKEN environment variable must be set"
    }
    $env:CHARM_SERVER_DB_DATA_SOURCE = "libsql://${env:TURSO_HOST}?authToken=${env:TURSO_AUTH_TOKEN}"

    $serverType = [System.IO.Path]::GetFileNameWithoutExtension($MyInvocation.MyCommand.Name) -replace '(?i)^start-server-', '' -replace '-', ' ' -replace ',', ' '

    Write-Verbose "serverType: $serverType"

    Write-Verbose "./identity serve $serverType"

    Invoke-Expression "./identity serve $serverType"
  }
  finally {
    Write-Verbose "Set-Location $cwd"

    Set-Location $cwd
  }
}
finally {
  Write-Verbose "Resetting VerbosePreference to $originalVerbosePreference"
  $VerbosePreference = $originalVerbosePreference
}
