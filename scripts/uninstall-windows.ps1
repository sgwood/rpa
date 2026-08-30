param(
  [Parameter(Mandatory = $true)]
  [string]$Executable,
  [switch]$PurgeData
)

$ErrorActionPreference = "Stop"
$taskName = "AI RPA Node"
if (Test-Path -LiteralPath $Executable) {
  & $Executable uninstall-hooks
}
if (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue) {
  Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
  Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
}

if ($PurgeData) {
  $dataDir = Join-Path $env:LOCALAPPDATA "stargold\ai-rpa\data"
  if (Test-Path -LiteralPath $dataDir) {
    Remove-Item -LiteralPath $dataDir -Recurse -Force
    Write-Host "已删除本机任务数据库和诊断缓存：$dataDir"
  }
} else {
  Write-Host "已移除后台节点与 Hook；本机任务数据保留。使用 -PurgeData 才会删除数据。"
}
