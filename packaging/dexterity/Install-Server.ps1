<#
.SYNOPSIS
  Installs the Dexterity Sheets server on the office PC that will hold the shared workbooks.

.DESCRIPTION
  Run once, in an elevated PowerShell, from the folder that holds DexSheetsServer.exe:
    powershell -ExecutionPolicy Bypass -File .\Install-Server.ps1
  It
    1. copies DexSheetsServer.exe to "C:\Program Files\Dexterity Sheets Server",
    2. creates the first administrator account if there is none (asks for a password),
    3. opens TCP 47810 (workbooks) and UDP 47811 (discovery) in Windows Firewall for the
       Domain and Private profiles only,
    4. registers a scheduled task that starts the server at boot as SYSTEM and restarts it if it
       stops.
  Data (accounts, workbooks, edit history, versions) lives in
  "C:\ProgramData\Dexterity Sheets\Server". Back that folder up.
#>
param(
  [string] $InstallDir = "$env:ProgramFiles\Dexterity Sheets Server",
  [string] $DataDir = "$env:ProgramData\Dexterity Sheets\Server",
  [int] $Port = 47810
)
$ErrorActionPreference = 'Stop'
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'Run this from an elevated (Run as administrator) PowerShell.'
}
$exe = Join-Path $PSScriptRoot 'DexSheetsServer.exe'
if (-not (Test-Path $exe)) { throw "DexSheetsServer.exe not found next to this script ($PSScriptRoot)." }

Get-ScheduledTask -TaskName 'Dexterity Sheets Server' -ErrorAction SilentlyContinue | Stop-ScheduledTask -ErrorAction SilentlyContinue
Get-Process DexSheetsServer -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $InstallDir, $DataDir | Out-Null
Copy-Item $exe $InstallDir -Force
$target = Join-Path $InstallDir 'DexSheetsServer.exe'

# First administrator.
if (-not (Test-Path (Join-Path $DataDir 'users.json'))) {
  Write-Host 'Create the first administrator account.'
  $name = Read-Host 'Administrator user name (e.g. vikram)'
  & $target --data $DataDir user add $name --role admin
  if ($LASTEXITCODE -ne 0) { throw 'Could not create the administrator.' }
}

# Firewall: LAN profiles only.
foreach ($r in @(@{n='Dexterity Sheets Server (TCP)'; p='TCP'; port=$Port}, @{n='Dexterity Sheets Discovery (UDP)'; p='UDP'; port=47811})) {
  Get-NetFirewallRule -DisplayName $r.n -ErrorAction SilentlyContinue | Remove-NetFirewallRule
  New-NetFirewallRule -DisplayName $r.n -Direction Inbound -Action Allow -Protocol $r.p -LocalPort $r.port -Profile Domain,Private -Program $target | Out-Null
}

# Start at boot, restart on failure.
$action = New-ScheduledTaskAction -Execute $target -Argument "--data `"$DataDir`" --port $Port"
$trigger = New-ScheduledTaskTrigger -AtStartup
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) -ExecutionTimeLimit ([TimeSpan]::Zero)
$who = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
Register-ScheduledTask -TaskName 'Dexterity Sheets Server' -Action $action -Trigger $trigger -Settings $settings -Principal $who -Force | Out-Null
Start-ScheduledTask -TaskName 'Dexterity Sheets Server'

Write-Host ''
Write-Host "Dexterity Sheets Server is running on $env:COMPUTERNAME (TCP $Port)."
Write-Host "Add teammates:  & '$target' --data '$DataDir' user add <name> --role editor"
Write-Host "List accounts:  & '$target' --data '$DataDir' user list"
Write-Host "Server log:     $DataDir\server.log"
