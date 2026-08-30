param(
  [Parameter(Mandatory = $true)]
  [string]$Executable
)

$ErrorActionPreference = "Stop"
$resolved = (Resolve-Path -LiteralPath $Executable).Path
$taskName = "AI RPA Node"
$action = New-ScheduledTaskAction -Execute $resolved -Argument "serve"
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Days 3650) -RestartCount 5 -RestartInterval (New-TimeSpan -Minutes 1)
$principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Limited
Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Settings $settings -Principal $principal -Force | Out-Null
Start-ScheduledTask -TaskName $taskName
& $resolved install-hooks

Write-Host "AI RPA 节点和四工具 Hook 已安装。"
Write-Host "健康检查：& '$resolved' doctor"
