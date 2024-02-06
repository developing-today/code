#!/usr/bin/env pwsh

Set-StrictMode -Version Latest

$PSNativeCommandUseErrorActionPreference = $true

if ($PSNativeCommandUseErrorActionPreference) {
  # always true, this is a linter workaround
  $ErrorActionPreference = "Stop"
  $PSDefaultParameterValues['*:ErrorAction'] = 'Stop'
}

$cwd = Get-Location

try {

  Set-Location $PSScriptRoot

  go mod tidy
  go build -v -tags libsql

} finally {
  Set-Location $cwd
}
