#!/usr/bin/env pwsh
param(
  [switch]$ForceInstallTempl,
  [switch]$Update,
  [switch]$SkipBuildWebJs,
  [switch]$SkipBuildTempl,
  [switch]$SkipBuildGoGenerate,
  [switch]$SkipBuildGoModTidy,
  [switch]$SkipBuildGoGet,
  [switch]$SkipBuildGoBuild,
  [switch]$SkipBuildGoExperiment
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

try {
  . "$PSScriptRoot\import-lib.ps1"

  $cwd = Get-Location

  Write-Verbose "Current directory: $cwd"

  try {
    if (-not $SkipBuildWebJs) {

      Write-Verbose "Set-Location $PSScriptRoot/web"

      Set-Location $PSScriptRoot/web

      if ($Update) {
        Write-Verbose "npm install -g npm"
        npm install -g npm
        Write-Verbose "npm install -g npm-check-updates"
        npm install -g npm-check-updates
        Write-Verbose "ncu -g"
        ncu -g
        Write-Verbose "ncu -u"
        ncu -u
        Write-Verbose "sleep 1"
        Start-Sleep 1
        Write-Verbose "npm install"
        npm install
      } else {
        Write-Verbose "npm ci"
        npm ci
      }

      Write-Verbose "npm run build"
      npm run build
    }

    Write-Verbose "Set-Location $PSScriptRoot"
    Set-Location $PSScriptRoot

    if (-not $SkipBuildGoExperiment) {
      if ([string]::IsNullOrEmpty($env:GOEXPERIMENT)) {
        $env:GOEXPERIMENT = 'rangefunc'
        Write-Verbose "Setting GOEXPERIMENT to $env:GOEXPERIMENT"
      }
      Write-Verbose "GOEXPERIMENT: $env:GOEXPERIMENT"
    }

    if ($Update -and -not $SkipBuildGoGet) {
      Write-Verbose "go get -u ./..."
      go get -u ./...
    } else {
      if ($SkipBuildGoGet) {
        Write-Verbose "Skipping go get"
      }
    }

    if (-not $SkipBuildGoModTidy) {
      Write-Verbose "go mod tidy"
      go mod tidy
    } else {
      Write-Verbose "Skipping go mod tidy"
    }

    if (-not $SkipBuildTempl) {

      Install-Templ -Force:$ForceInstallTempl
      $goBinPath = Join-Path $(go env GOPATH) "bin"
      Write-Verbose "goBinPath: $goBinPath"
      $templCommand = Join-Path $goBinPath "templ"
      Write-Verbose "templCommand: $templCommand"

      Write-Verbose "templ fmt"
      ."$templCommand" fmt .

      Write-Verbose "templ generate"
      $generateOutput = ."$templCommand" generate
      Write-Verbose "templ generate output:"
      $generateOutput -split "`n" | ForEach-Object { Write-Verbose ($_ -replace "\(Γ£ù\)", "❌" -replace "\(Γ£ô\)", "✅") }

      if ($generateOutput -match '✗' -or $generateOutput -match 'Γ£ù') {
        Write-Verbose "templ generate failed"
        throw "templ generate failed"
      }
      else {
        Write-Verbose "templ generate succeeded"
      }
    } else {
      Write-Verbose "Skipping templ build"
    }

    if (-not $SkipBuildGoGenerate) {
      Write-Verbose "go generate ./..."
      go generate ./...
    }

    if (-not $SkipBuildGoBuild) {
      $tags = [System.IO.Path]::GetFileNameWithoutExtension($MyInvocation.MyCommand.Name) -replace '(?i)^build-', '' -replace '-', ',' -replace ' ', ','

      Write-Verbose "tags: $tags"

      Write-Verbose "go build -v -tags $tags"
      go build -v -tags $tags
    } else {
      Write-Verbose "Skipping go build"
    }
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
