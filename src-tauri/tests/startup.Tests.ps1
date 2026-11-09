# Safe behavior tests: load function definitions only; replace all Task Scheduler
# and registry-facing functions before executing the production operation switch.
$ErrorActionPreference = 'Stop'
$tokens = $null
$errors = $null
$path = Join-Path $PSScriptRoot '..\src\startup.ps1'
$ast = [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
foreach ($function in $ast.FindAll({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    . ([ScriptBlock]::Create($function.Extent.Text))
}
function Assert($condition, [string]$message) { if (!$condition) { throw $message } }

$exe = 'C:\Apps with spaces\Tiếng Việt\power-controle.exe'
Assert (Test-OwnedCommand ('"' + $exe + '" --autostart')) 'Quoted Unicode path was not recognized'
Assert (Test-OwnedCommand 'C:\Old installation\power-controle.exe') 'Old app installation was not recognized'
Assert (!(Test-OwnedCommand '"C:\Other\not-power-controle.exe"')) 'Unrelated executable was considered owned'
Assert (!(Test-OwnedCommand 'cmd.exe /c C:\Apps\power-controle.exe')) 'Wrapper command was considered owned'
Assert (!(Test-OwnedCommand '"C:\Other\unrelated.exe" --name power-controle.exe')) 'Argument substring was considered owned'
Assert (!(Test-OwnedCommand '')) 'Empty command was considered owned'
Assert (!(Test-TaskEnabled $null)) 'Missing task was considered enabled'
Assert (!(Test-TaskEnabled ([PSCustomObject]@{ Settings = @{ Enabled = $false }; Triggers = @(@{ Enabled = $true }) }))) 'Disabled task was considered enabled'
Assert (!(Test-TaskEnabled ([PSCustomObject]@{ Settings = @{ Enabled = $true }; Triggers = @(@{ Enabled = $false }) }))) 'Disabled trigger was considered enabled'

Assert (!(Test-ProtectedLocation $exe)) 'User-writable startup was accepted'
Assert (Test-ProtectedLocation (Join-Path $env:ProgramFiles 'Power Control\power-controle.exe')) 'Program Files installation was rejected'
Assert (!(Test-ProtectedLocation ($env:ProgramFiles + '-fake\power-controle.exe'))) 'Program Files prefix spoof was accepted'
$exe = Join-Path $env:ProgramFiles 'Tiếng Việt\Power Control\power-controle.exe'

$script:events = [Collections.Generic.List[string]]::new()
$script:task = $null
$script:legacy = @()
$script:failRegistration = $false
$sid = 'S-1-5-21-test'
$taskName = "PowerControl-$sid"
function Get-OwnedTask { return $script:task }
function Get-LegacyEntries { return $script:legacy }
function Remove-LegacyEntries($entries) { $script:events.Add('cleanup') }
function New-ScheduledTaskAction($Execute, $Argument, $WorkingDirectory) {
    Assert ($Execute -ceq $exe) 'Executable changed or was escaped as code'
    Assert ($Argument -eq '--autostart') 'Missing login argument'
    Assert ($WorkingDirectory -eq [IO.Path]::GetDirectoryName($exe)) 'Wrong working directory'
    return @{}
}
function New-ScheduledTaskTrigger([switch]$AtLogOn, $User) {
    Assert ($AtLogOn -and $User -eq $sid) 'Trigger is not scoped to user login'
    return [PSCustomObject]@{ Delay = ''; Enabled = $true }
}
function New-ScheduledTaskPrincipal($UserId, $LogonType, $RunLevel) {
    Assert ($UserId -eq $sid -and $LogonType -eq 'Interactive' -and $RunLevel -eq 'Highest') 'Wrong task session or privileges'
    return @{}
}
function New-ScheduledTaskSettingsSet($MultipleInstances, $ExecutionTimeLimit, [switch]$AllowStartIfOnBatteries, [switch]$DontStopIfGoingOnBatteries, [switch]$StartWhenAvailable) {
    Assert ($MultipleInstances -eq 'IgnoreNew') 'Duplicate instances are allowed'
    Assert ($ExecutionTimeLimit -eq [TimeSpan]::Zero) 'Task has an execution timeout'
    Assert ($AllowStartIfOnBatteries -and $DontStopIfGoingOnBatteries -and $StartWhenAvailable) 'Battery or availability restriction'
    return @{ Enabled = $true }
}
function Register-ScheduledTask($TaskName, $TaskPath, $Action, $Trigger, $Principal, $Settings, $Description, [switch]$Force) {
    Assert ($TaskName -eq "PowerControl-$sid" -and $TaskPath -eq '\') 'Wrong task identity'
    Assert ($Trigger.Delay -eq 'PT15S') 'Wrong login delay'
    $script:events.Add('register')
    if ($script:failRegistration) { throw 'Simulated access denied' }
    $script:task = [PSCustomObject]@{ Settings = $Settings; Triggers = @($Trigger); Actions = @([PSCustomObject]@{ Execute = $exe; Arguments = '--autostart' }) }
}
function Unregister-ScheduledTask($TaskName, $TaskPath, $Confirm) {
    Assert ($TaskName -eq "PowerControl-$sid") 'Unregistered an unrelated task'
    $script:events.Add('unregister')
    $script:task = $null
}
# Guard against accidentally using real commands if this harness is edited.
Assert ((Get-Command Register-ScheduledTask).CommandType -eq 'Function') 'Registration must be mocked'
Assert ((Get-Command Unregister-ScheduledTask).CommandType -eq 'Function') 'Unregistration must be mocked'
$tryNode = $ast.EndBlock.Statements | Where-Object { $_ -is [System.Management.Automation.Language.TryStatementAst] } | Select-Object -Last 1
$body = $tryNode.Body.Extent.Text
$operations = [ScriptBlock]::Create($body.Substring(1, $body.Length - 2))

$operation = 'query'
Assert ((. $operations) -eq 'disabled') 'Missing startup should be disabled'
Assert ($events.Count -eq 0) 'Query changed startup state'
$script:legacy = @([PSCustomObject]@{ Enabled = $true })
Assert ((. $operations) -eq 'enabled') 'Enabled legacy entry not reported'
$operation = 'migrate'
Assert ((. $operations) -eq 'ok') 'Migration failed'
Assert (($events -join ',') -eq 'register,cleanup') 'Migration cleanup occurred before registration'
$events.Clear()
$script:task.Actions[0].Execute = 'C:\Old location\power-controle.exe'
$operation = 'query'
Assert ((. $operations) -eq 'disabled') 'Stale executable was reported working'
$operation = 'migrate'
. $operations | Out-Null
Assert (($events -join ',') -eq 'register,cleanup') 'Moved executable was not repaired before cleanup'
$events.Clear()
$script:task.Actions[0].Execute = 'C:\Old location\power-controle.exe'
$script:failRegistration = $true
$failed = $false
try { . $operations | Out-Null } catch { $failed = $true }
Assert $failed 'Stale path repair failure was swallowed'
Assert (($events -join ',') -eq 'register') 'Fallback removed after failed stale path repair'
$script:failRegistration = $false
$events.Clear()
$script:task.Settings.Enabled = $false
$operation = 'query'
Assert ((. $operations) -eq 'disabled') 'Disabled task did not take precedence'
$operation = 'migrate'
. $operations | Out-Null
Assert (($events -join ',') -eq 'cleanup') 'Migration re-enabled an explicitly disabled task'

$events.Clear()
$script:task = $null
$script:failRegistration = $true
$operation = 'migrate'
$failed = $false
try { . $operations | Out-Null } catch { $failed = $true }
Assert $failed 'Registration failure was swallowed'
Assert (($events -join ',') -eq 'register') 'Legacy removed after registration failed'
$script:failRegistration = $false
$events.Clear()
$script:legacy = @([PSCustomObject]@{ Enabled = $false })
. $operations | Out-Null
Assert ($events.Count -eq 0) 'Disabled legacy entry was migrated'
$operation = 'enable'
. $operations | Out-Null
$events.Clear()
$operation = 'disable'
Assert ((. $operations) -eq 'ok') 'Disable failed'
Assert (($events -join ',') -eq 'unregister,cleanup') 'Disable did not clean up both mechanisms'
'PASS: ownership, Unicode paths, task settings/session, status, migration ordering/failure, disabled choices, enable/disable (mocked; no machine changes)'
