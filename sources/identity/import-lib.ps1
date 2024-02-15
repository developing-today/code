#!/usr/bin/env pwsh

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

$caughtError = $null

try {
  $cwd = Get-Location

  Write-Verbose "Current directory: $cwd"

  try {

    Write-Verbose "Set-Location $PSScriptRoot"

    Set-Location $PSScriptRoot

    function Install-Templ {
      param(
        [switch]$Force
      )
      $templCommand = "templ"

      try {
        $output = & $templCommand --version
        Write-Verbose "templ is already installed. Version: $output"
        if (-not $Force) {
          return
        } else {
          Write-Verbose "Force flag is set, attempting to reinstall..."
        }
      }
      catch {
        Write-Verbose "templ is not installed, attempting to install..."
      }

      Write-Verbose "go install github.com/a-h/templ/cmd/templ@latest"
      go install github.com/a-h/templ/cmd/templ@latest

      $goBinPath = "$(go env GOPATH)\bin"
      if (-Not $goBinPath) {
        $goBinPath = go env GOBIN
      }
      Write-Verbose "goBinPath: $goBinPath"

      if (-Not ($env:PATH -split ';' -contains $goBinPath)) {
        Write-Verbose "Adding Go bin path to the current session PATH..."
        $env:PATH += ";$goBinPath"
      }

      try {
        $output = & $templCommand --version
        Write-Verbose "Successfully installed templ. Version: $output"
      }
      catch {
        Write-Verbose "Failed to install templ."
      }
    }
  }
  catch {
    Write-Error $_
    $caughtError = $_
  }
  finally {
    Write-Verbose "Set-Location $cwd"

    Set-Location $cwd

    if ($null -ne $caughtError) {
      Write-Error "An error occurred: $caughtError"
      throw $caughtError
    }
  }
}
finally {
  Write-Verbose "Resetting VerbosePreference to $originalVerbosePreference"
  $VerbosePreference = $originalVerbosePreference

  if ($null -ne $caughtError) {
    Write-Error "An error occurred: $caughtError"
    throw $caughtError
  }
}
