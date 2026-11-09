# Paths are passed as environment data, never interpolated into PowerShell code.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$sid = $identity.User.Value
$taskName = "PowerControl-$sid"
$exe = $env:POWER_CONTROL_EXE
$operation = $env:POWER_CONTROL_OPERATION
$runKey = 'Software\Microsoft\Windows\CurrentVersion\Run'
$approvedKey = 'Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
$valueName = 'power-controle'

function Test-OwnedCommand([string]$command) {
    if ([string]::IsNullOrWhiteSpace($command)) { return $false }
    $command = [Environment]::ExpandEnvironmentVariables($command.Trim())
    # Match the entire executable token, not a substring in another command.
    if ($command -match '^"([^"]+)"(?:\s|$)') { $target = $Matches[1] }
    elseif ($command -match '^(.+?\.exe)(?:\s|$)') { $target = $Matches[1] }
    else { return $false }
    return ($target -ieq $exe -or [IO.Path]::GetFileName($target) -ieq 'power-controle.exe')
}

function Get-LegacyEntries {
    foreach ($hive in @([Microsoft.Win32.Registry]::CurrentUser, [Microsoft.Win32.Registry]::LocalMachine)) {
        $key = $hive.OpenSubKey($runKey)
        if ($null -eq $key) { continue }
        try { $command = $key.GetValue($valueName) } finally { $key.Dispose() }
        if (!(Test-OwnedCommand $command)) { continue }
        $enabled = $true
        $approved = $hive.OpenSubKey($approvedKey)
        if ($null -ne $approved) {
            try { $bytes = $approved.GetValue($valueName) } finally { $approved.Dispose() }
            if ($bytes -is [byte[]] -and $bytes.Length -gt 0) {
                # StartupApproved uses even values for enabled, odd for disabled.
                $enabled = (($bytes[0] -band 1) -eq 0)
            }
        }
        [PSCustomObject]@{ Hive = $hive; Enabled = $enabled }
    }
}

function Get-OwnedTask {
    # Enumeration distinguishes an absent task from access/service errors.
    Get-ScheduledTask -ErrorAction Stop |
        Where-Object { $_.TaskPath -eq '\' -and $_.TaskName -eq $taskName } | Select-Object -First 1
}

function Test-TaskIntent($task) {
    if ($null -eq $task -or !$task.Settings.Enabled) { return $false }
    return (@($task.Triggers | Where-Object { $_.Enabled }).Count -gt 0)
}

function Test-TaskEnabled($task) {
    return ((Test-TaskIntent $task) -and @($task.Actions | Where-Object { $_.Execute -ieq $exe -and $_.Arguments -eq '--autostart' }).Count -gt 0)
}

function Test-ProtectedLocation([string]$path) {
    # Never create a silent elevated launch of a Desktop/Downloads/per-user exe:
    # another unelevated process could replace it before the next sign-in.
    $fullPath = [IO.Path]::GetFullPath($path)
    foreach ($root in @($env:ProgramFiles, ${env:ProgramFiles(x86)})) {
        if (!$root) { continue }
        $prefix = [IO.Path]::GetFullPath($root).TrimEnd('\') + '\'
        if ($fullPath.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) { return $true }
    }
    return $false
}

function Register-StartupTask {
    if (!(Test-ProtectedLocation $exe)) {
        throw 'Install Power Control for all users under Program Files before enabling elevated startup. Startup from a user-writable folder is unsafe.'
    }
    $action = New-ScheduledTaskAction -Execute $exe -Argument '--autostart' -WorkingDirectory ([IO.Path]::GetDirectoryName($exe))
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $sid
    $trigger.Delay = 'PT15S'
    # Interactive (not SYSTEM/S4U) is essential: the tray belongs to this desktop.
    $principal = New-ScheduledTaskPrincipal -UserId $sid -LogonType Interactive -RunLevel Highest
    $settings = New-ScheduledTaskSettingsSet -MultipleInstances IgnoreNew -ExecutionTimeLimit ([TimeSpan]::Zero) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable
    Register-ScheduledTask -TaskName $taskName -TaskPath '\' -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Description 'Power Control tray at user sign-in' -Force | Out-Null
    if (!(Test-TaskEnabled (Get-OwnedTask))) { throw 'The startup task was created but is not enabled.' }
}

function Remove-LegacyEntries($entries) {
    foreach ($entry in $entries) {
        $key = $entry.Hive.OpenSubKey($runKey, $true)
        if ($null -ne $key) {
            try {
                # Recheck ownership immediately before deleting.
                if (Test-OwnedCommand ($key.GetValue($valueName))) { $key.DeleteValue($valueName, $false) }
            } finally { $key.Dispose() }
        }
        # Leave StartupApproved metadata alone: it cannot start an application,
        # and retaining it preserves Task Manager's previous disabled choice.
    }
}

try {
    $task = Get-OwnedTask
    $legacy = @(Get-LegacyEntries)
    switch ($operation) {
        'query' {
            # An explicitly disabled task takes precedence over stale Run entries.
            $enabled = if ($null -ne $task) { Test-TaskEnabled $task } else { @($legacy | Where-Object { $_.Enabled }).Count -gt 0 }
            if ($enabled) { 'enabled' } else { 'disabled' }
        }
        'enable' {
            Register-StartupTask
            # Never remove the old mechanism before the replacement succeeds.
            Remove-LegacyEntries $legacy
            'ok'
        }
        'disable' {
            if ($null -ne $task) {
                Unregister-ScheduledTask -TaskName $taskName -TaskPath '\' -Confirm:$false
            }
            Remove-LegacyEntries $legacy
            'ok'
        }
        'migrate' {
            if ($null -eq $task -and @($legacy | Where-Object { $_.Enabled }).Count -gt 0) {
                Register-StartupTask
                Remove-LegacyEntries $legacy
            } elseif ($null -ne $task) {
                if ((Test-TaskIntent $task) -and !(Test-TaskEnabled $task)) {
                    # Repair a moved installation before deleting its fallback.
                    Register-StartupTask
                }
                # Do not override an explicitly disabled scheduled task.
                Remove-LegacyEntries $legacy
            }
            'ok'
        }
        default { throw 'Invalid startup operation.' }
    }
} catch {
    [Console]::Error.WriteLine("Startup configuration failed: $($_.Exception.Message). Open the installed Power Control app as administrator and enable Start with Windows again.")
    exit 1
}
