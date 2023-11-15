$currentDir = (Get-Location).Path

try {
  Set-Location "$PSScriptRoot/src/graph"

  try {
  .'go' 'run' 'main.go'

  } catch {
    Write-Host $_.Exception.Message

  } finally {
    Write-Host 'Done'
  }

} finally {
  Set-Location $currentDir
}
