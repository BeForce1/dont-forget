#Requires -Version 7
<#
.SYNOPSIS
  Control panel for Don't Forget.
  ↑/↓ move, Enter runs an item, ←/→ change a setting, Q or Esc quits.
.PARAMETER Status
  Print the current state and exit (no menu).
#>
param([switch]$Status)

$ErrorActionPreference = 'Stop'
$Root = Split-Path $PSScriptRoot -Parent
$Exe = Join-Path $Root 'app\DontForget.exe'
$Built = Join-Path $Root 'target\release\DontForget.exe'
$Cargo = Join-Path $HOME '.cargo\bin\cargo.exe'
$SettingsPath = Join-Path $env:APPDATA 'DontForget\settings.ini'
$RunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$Repo = 'https://github.com/BeForce1/dont-forget'

$E = [char]27
$C = @{
    Reset = "$E[0m"; Bold = "$E[1m"; Dim = "$E[90m"; White = "$E[97m"
    Green = "$E[38;2;46;204;113m"; Blue = "$E[38;2;52;152;255m"; Amber = "$E[38;2;255;186;0m"
    Red = "$E[38;2;229;72;77m"; Sel = "$E[48;2;38;38;44m"
}

if (-not ('DontForget.Win' -as [type])) {
    Add-Type -Namespace DontForget -Name Win -UsingNamespace System.Text -MemberDefinition @'
public delegate bool EnumProc(IntPtr h, IntPtr l);
[DllImport("user32.dll")] static extern bool EnumWindows(EnumProc p, IntPtr l);
[DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
[DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
public static int CountNotes() {
    int c = 0;
    EnumWindows((h, _) => {
        var s = new StringBuilder(64);
        GetWindowText(h, s, 64);
        if (IsWindowVisible(h) && s.ToString() == "Don't Forget note") c++;
        return true;
    }, IntPtr.Zero);
    return c;
}
'@
}

# ---------- settings (same format the app writes) ----------

$AfterModes = [ordered]@{ follow = 'Follow my mouse'; drop = 'Follow until I click'; stay = 'Stay where I snipped' }
$HoldTimes = 150, 250, 400, 600

function Read-Settings {
    $s = [ordered]@{ after_capture = 'follow'; hold_ms = '250'; ignored_apps = 'SLDWORKS,blender,FreeCAD,acad'; shown_intro = '0' }
    if (Test-Path $SettingsPath) {
        foreach ($line in Get-Content $SettingsPath) {
            if ($line -match '^\s*([a-z_]+)\s*=\s*([^;]*)') { $s[$Matches[1]] = $Matches[2].Trim() }
        }
    }
    $s
}

function Save-Settings($s) {
    New-Item -ItemType Directory -Force (Split-Path $SettingsPath) | Out-Null
    $lines = @(
        "; Don't Forget settings. Changes apply when you close Notepad."
        "after_capture=$($s.after_capture)   ; follow | drop | stay"
        "hold_ms=$($s.hold_ms)   ; how long to hold the wheel before snipping"
        "ignored_apps=$($s.ignored_apps)   ; process names where the wheel is left alone"
        "shown_intro=$($s.shown_intro)"
    )
    [IO.File]::WriteAllText($SettingsPath, ($lines -join "`n") + "`n")   # LF, like the app writes
}

# ---------- state ----------

function Get-AppProcess { Get-Process DontForget -ErrorAction SilentlyContinue | Select-Object -First 1 }

function Get-State {
    $p = Get-AppProcess
    [pscustomobject]@{
        Running  = [bool]$p
        Id       = $p.Id
        RamMB    = if ($p) { [math]::Round($p.PrivateMemorySize64 / 1MB, 1) } else { 0 }
        Notes    = if ($p) { [DontForget.Win]::CountNotes() } else { 0 }
        Startup  = [bool](Get-ItemProperty $RunKey -Name DontForget -ErrorAction SilentlyContinue)
        Settings = Read-Settings
        HasExe   = Test-Path $Exe
    }
}

# ---------- actions ----------

$script:Message = ''
function Say($text, $color = $C.Green) { $script:Message = "$color$text$($C.Reset)" }

function Confirm-Key($question) {
    Write-Host "`n  $($C.Amber)$question$($C.Reset) $($C.Dim)[y/N]$($C.Reset) " -NoNewline
    $k = [Console]::ReadKey($true)
    $k.Key -eq 'Y'
}

function Start-App {
    if (Get-AppProcess) { return Say 'Already running.' $C.Dim }
    if (-not (Test-Path $Exe)) { return Say 'No app\DontForget.exe yet. Use "Rebuild from source" first.' $C.Amber }
    Start-Process $Exe
    Start-Sleep -Milliseconds 400
    Say 'Started. Hold the wheel and drag to snip.'
}

function Stop-App([switch]$Quiet) {
    $p = Get-AppProcess
    if (-not $p) { if (-not $Quiet) { Say 'Not running.' $C.Dim }; return $true }
    $notes = [DontForget.Win]::CountNotes()
    if ($notes -gt 0 -and -not (Confirm-Key "$notes note(s) open will be lost. Stop anyway?")) {
        Say 'Kept running.' $C.Dim
        return $false
    }
    Stop-Process -Id $p.Id   # by PID only, never by image name
    $p.WaitForExit(3000) | Out-Null
    if (-not $Quiet) { Say 'Stopped. The mouse wheel is back to normal.' }
    $true
}

function Restart-App {
    if (Stop-App -Quiet) { Start-App; Say 'Restarted with the current settings.' }
}

function Switch-Startup {
    if (Get-ItemProperty $RunKey -Name DontForget -ErrorAction SilentlyContinue) {
        Remove-ItemProperty $RunKey -Name DontForget
        Say 'Will not start with Windows.'
    } else {
        Set-ItemProperty $RunKey -Name DontForget -Value "`"$Exe`""
        Say 'Will start with Windows.'
    }
}

function Set-Setting($name, $value) {
    $s = Read-Settings
    $s[$name] = "$value"
    Save-Settings $s
    if (Get-AppProcess) { Say 'Saved. Restart the app to apply it.' $C.Amber } else { Say 'Saved.' }
}

function Step-After($dir) {
    $keys = @($AfterModes.Keys)
    $i = [array]::IndexOf($keys, (Read-Settings).after_capture)
    Set-Setting after_capture $keys[($i + $dir + $keys.Count) % $keys.Count]
}

function Step-Hold($dir) {
    $i = [array]::IndexOf($HoldTimes, [int](Read-Settings).hold_ms)
    if ($i -lt 0) { $i = 1 }
    Set-Setting hold_ms $HoldTimes[[math]::Clamp($i + $dir, 0, $HoldTimes.Count - 1)]
}

function Edit-Ignored {
    $apps = [System.Collections.Generic.List[string]]((Read-Settings).ignored_apps -split ',' | Where-Object { $_ })
    [Console]::CursorVisible = $true
    Clear-Host
    Write-Host "`n  $($C.Bold)Ignored apps$($C.Reset)  $($C.Dim)the wheel is left alone over these (process names, no .exe)$($C.Reset)`n"
    while ($true) {
        Write-Host "  $($C.White)$($apps -join ', ')$($C.Reset)"
        $in = Read-Host "  Type +name to add, -name to remove, Enter when done"
        if (-not $in) { break }
        foreach ($t in $in -split '[ ,]+' | Where-Object { $_ }) {
            $name = $t.Substring(1) -replace '\.exe$', ''
            if ($t[0] -eq '+' -and $name -and -not ($apps -contains $name)) { $apps.Add($name) }
            elseif ($t[0] -eq '-') { $apps.RemoveAll({ param($a) $a -eq $name }) | Out-Null }
        }
    }
    [Console]::CursorVisible = $false
    Set-Setting ignored_apps ($apps -join ',')
    Clear-Host
}

function Build-App {
    [Console]::CursorVisible = $true
    Clear-Host
    if (-not (Test-Path $Cargo)) { Say "Rust isn't installed ($Cargo)." $C.Red; return }
    Write-Host "`n  $($C.Bold)Building release…$($C.Reset)`n"
    Push-Location $Root
    try { & $Cargo build --release 2>&1 | ForEach-Object { "  $_" } | Write-Host; $ok = $LASTEXITCODE -eq 0 }
    finally { Pop-Location }
    if (-not $ok) { Say 'Build failed. See the output above.' $C.Red; Read-Host "`n  Enter to go back" | Out-Null; Clear-Host; return }
    $was = [bool](Get-AppProcess)
    if ($was -and -not (Stop-App -Quiet)) { Say 'Built, but not installed (app kept running).' $C.Amber; Clear-Host; return }
    New-Item -ItemType Directory -Force (Split-Path $Exe) | Out-Null
    Copy-Item $Built $Exe -Force
    if ($was) { Start-App }
    Say ("Built and installed ({0:N0} KB){1}." -f ((Get-Item $Exe).Length / 1KB), $(if ($was) { ', restarted' } else { '' }))
    [Console]::CursorVisible = $false
    Clear-Host
}

# ---------- menu ----------

$Items = @(
    @{ Key = 'S'; Label = { param($s) if ($s.Running) { 'Stop' } else { 'Start' } }; Run = { if (Get-AppProcess) { Stop-App | Out-Null } else { Start-App } } }
    @{ Key = 'R'; Label = { 'Restart' }; Run = { Restart-App } }
    @{ Key = 'W'; Label = { param($s) "Start with Windows   $(if ($s.Startup) { "$($C.Green)on" } else { "$($C.Dim)off" })$($C.Reset)" }; Run = { Switch-Startup }; Step = { Switch-Startup } }
    @{ Key = 'A'; Label = { param($s) "After a snip         $($C.Blue)‹ $($AfterModes[$s.Settings.after_capture]) ›$($C.Reset)" }; Run = { Step-After 1 }; Step = { param($d) Step-After $d } }
    @{ Key = 'H'; Label = { param($s) "Hold time            $($C.Blue)‹ $($s.Settings.hold_ms) ms ›$($C.Reset)" }; Run = { Step-Hold 1 }; Step = { param($d) Step-Hold $d } }
    @{ Key = 'I'; Label = { param($s) "Ignored apps         $($C.Dim)$(@($s.Settings.ignored_apps -split ',' | Where-Object { $_ }).Count) apps$($C.Reset)" }; Run = { Edit-Ignored } }
    @{ Key = 'E'; Label = { 'Open settings file' }; Run = { Start-Process notepad.exe $SettingsPath; Say 'Opened in Notepad.' } }
    @{ Key = 'B'; Label = { 'Rebuild from source' }; Run = { Build-App } }
    @{ Key = 'G'; Label = { 'Open GitHub page' }; Run = { Start-Process $Repo; Say 'Opened in your browser.' } }
    @{ Key = 'Q'; Label = { 'Quit' }; Run = { $script:Quit = $true } }
)

function Show-Status($s) {
    $dot = if ($s.Running) { "$($C.Green)● Running$($C.Reset)" } else { "$($C.Dim)○ Stopped$($C.Reset)" }
    $detail = if ($s.Running) { "$($C.Dim)pid $($s.Id) · $($s.RamMB) MB · $($s.Notes) note$(if ($s.Notes -ne 1) { 's' }) open$($C.Reset)" } else { '' }
    "  $dot   $detail"
}

function Render($s, $sel) {
    $out = [System.Text.StringBuilder]::new()
    $line = { param($t) [void]$out.Append("$t$E[K`n") }
    & $line ''
    & $line "  $($C.Amber)$($C.Bold)! Don't Forget$($C.Reset)  $($C.Dim)control panel$($C.Reset)"
    & $line "  $($C.Dim)$('─' * 46)$($C.Reset)"
    & $line (Show-Status $s)
    if (-not $s.HasExe) { & $line "  $($C.Amber)app\DontForget.exe is missing, use Rebuild$($C.Reset)" }
    & $line ''
    for ($i = 0; $i -lt $Items.Count; $i++) {
        $it = $Items[$i]
        $text = & $it.Label $s
        if ($i -eq $sel) { & $line "  $($C.Sel)$($C.Amber) › $($C.White)$text$($C.Sel)  $($C.Reset)  $($C.Dim)$($it.Key)$($C.Reset)" }
        else { & $line "     $text  $($C.Dim)$($it.Key)$($C.Reset)" }
    }
    & $line ''
    & $line "  $($C.Dim)↑↓ move · Enter run · ←→ change · letter = shortcut · Q quit$($C.Reset)"
    & $line ''
    & $line "  $script:Message"
    [void]$out.Append("$E[J")
    [Console]::SetCursorPosition(0, 0)
    [Console]::Write($out.ToString())
}

if ($Status) {
    $s = Get-State
    Show-Status $s
    "  start with Windows: $(if ($s.Startup) { 'on' } else { 'off' }) · after a snip: $($AfterModes[$s.Settings.after_capture]) · hold: $($s.Settings.hold_ms) ms · ignored: $($s.Settings.ignored_apps)"
    return
}

$sel = 0
$script:Quit = $false
$lastPoll = [datetime]::MinValue
$dirty = $true
try {
    [Console]::CursorVisible = $false
    Clear-Host
    while (-not $script:Quit) {
        if ($dirty -or ((Get-Date) - $lastPoll).TotalMilliseconds -ge 1000) {
            $state = Get-State
            Render $state $sel
            $lastPoll = Get-Date
            $dirty = $false
        }
        if (-not [Console]::KeyAvailable) { Start-Sleep -Milliseconds 60; continue }
        $k = [Console]::ReadKey($true)
        $dirty = $true
        $script:Message = ''
        switch ($k.Key) {
            'UpArrow' { $sel = ($sel - 1 + $Items.Count) % $Items.Count }
            'DownArrow' { $sel = ($sel + 1) % $Items.Count }
            'Enter' { & $Items[$sel].Run }
            'LeftArrow' { if ($Items[$sel].Step) { & $Items[$sel].Step -1 } }
            'RightArrow' { if ($Items[$sel].Step) { & $Items[$sel].Step 1 } }
            'Escape' { $script:Quit = $true }
            default {
                $hit = [array]::FindIndex([object[]]$Items, [Predicate[object]] { param($it) $it.Key -eq "$($k.KeyChar)".ToUpper() })
                if ($hit -ge 0) { $sel = $hit; & $Items[$hit].Run }
            }
        }
        if (-not $script:Quit) { Clear-Host }
    }
} finally {
    [Console]::CursorVisible = $true
    [Console]::Write("$($C.Reset)`n")
}
