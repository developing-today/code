#!/usr/bin/env pwsh
param(
  [switch]$ForceInstallTempl,
  [switch]$Update
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

Write-Verbose "originalVerbosePreference: $originalVerbosePreference"
Write-Verbose "VerbosePreference: $VerbosePreference"

try {

  $cwd = Get-Location

  Write-Verbose "Current directory: $cwd"

  try {

    Write-Verbose "Set-Location $PSScriptRoot"

    Set-Location $PSScriptRoot

    ."$PSScriptRoot/build-libsql.ps1" -ForceInstallTempl:$ForceInstallTempl -Update:$Update

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
