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
  . "$PSScriptRoot\import-lib.ps1"

  $cwd = Get-Location

  Write-Verbose "Current directory: $cwd"

  try {

    Write-Verbose "Set-Location $PSScriptRoot"

    Set-Location $PSScriptRoot

    if ([string]::IsNullOrEmpty($env:GOEXPERIMENT)) {
      $env:GOEXPERIMENT = 'rangefunc'
    }
    Write-Verbose "GOEXPERIMENT: $env:GOEXPERIMENT"

    if ($Update) {
      Write-Verbose "go get -u"
      go get -u
    }

    Write-Verbose "go mod tidy"
    go mod tidy

    Install-Templ -Force:$ForceInstallTempl

    Write-Verbose "templ fmt"
    templ fmt .

    Write-Verbose "templ generate"
    $generateOutput = templ generate
    Write-Verbose "templ generate output:"
    $generateOutput -split "`n" | ForEach-Object { Write-Verbose ($_ -replace "\(Γ£ù\)", "❌" -replace "\(Γ£ô\)", "✅") }

    if ($generateOutput -match '✗' -or $generateOutput -match 'Γ£ù') {
      Write-Verbose "templ generate failed"
      throw "templ generate failed"
    }
    else {
      Write-Verbose "templ generate succeeded"
    }

    Write-Verbose "go generate ./..."
    go generate ./...

    $tags = [System.IO.Path]::GetFileNameWithoutExtension($MyInvocation.MyCommand.Name) -replace '(?i)^build-', '' -replace '-', ',' -replace ' ', ','

    Write-Verbose "tags: $tags"

    Write-Verbose "go build -v -tags $tags"
    go build -v -tags $tags
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
