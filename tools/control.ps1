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
    $s = [ordered]@{ after_capture = 'follow'; hold_ms = '250'; ignored_apps = 'SLDWORKS,blender,FreeCAD,acad'; ignored_sites = ''; block_games = '1'; allowed_games = ''; shown_intro = '0' }
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
        "ignored_sites=$($s.ignored_sites)   ; websites where it is off, e.g. youtube.com"
        "block_games=$($s.block_games)   ; 1 = off in games unless listed in allowed_games"
        "allowed_games=$($s.allowed_games)   ; games where it still works"
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

function Switch-Games {
    Set-Setting block_games $(if ((Read-Settings).block_games -ne '0') { '0' } else { '1' })
}

function Step-After($dir) {
    $keys = @($AfterModes.Keys)
    $i = [array]::IndexOf($keys, (Read-Settings).after_capture)
    Set-Setting after_capture $keys[($i + $dir + $keys.Count) % $keys.Count]
}

function Step-Hold($dir) {
    $ms = 0
    [void][int]::TryParse((Read-Settings).hold_ms, [ref]$ms)   # a hand-edited typo must not crash the panel
    $i = [array]::IndexOf($HoldTimes, $ms)
    if ($i -lt 0) { $i = 1 }
    Set-Setting hold_ms $HoldTimes[[math]::Clamp($i + $dir, 0, $HoldTimes.Count - 1)]
}

# Same rules as the app (src/common/games.rs): game-store library folders + Windows' own game list.
$GameFolders = '\steamapps\common\', '\epic games\', '\gog galaxy\games\', '\gog games\', '\xboxgames\', '\riot games\',
    '\ubisoft game launcher\games\', '\ea games\', '\origin games\', '\rockstar games\', '\itch\apps\'
$Launchers = '\epic games\launcher\', '\epic games\epiconlineservices\'

function Test-GamePath($path) {
    $p = "$path".ToLower()
    foreach ($l in $Launchers) { if ($p.Contains($l)) { return $false } }
    foreach ($f in $GameFolders) { if ($p.Contains($f)) { return $true } }
    $false
}

function Get-KnownGames {
    Get-ChildItem HKCU:\System\GameConfigStore\Children -ErrorAction SilentlyContinue |
        ForEach-Object { (Get-ItemProperty $_.PSPath -Name MatchedExeFullPath -ErrorAction SilentlyContinue).MatchedExeFullPath } |
        Where-Object { $_ } | Sort-Object -Unique
}

function Get-AppCatalog {
    # Start menu shortcuts + registered App Paths + windows open right now + games Windows knows.
    $found = @{}
    $known = @(Get-KnownGames)
    $add = {
        param($exe, $name, $path)
        # some shortcuts point at a launcher (SOLIDWORKS: i386_SldWorks.exe starts SLDWORKS.exe)
        $stem = [IO.Path]::GetFileNameWithoutExtension("$exe") -replace '^i386_', ''
        if (-not $stem -or $stem -match '^(unins\d*|uninstall|setup|update|updater|crashpad_handler)$') { return }
        $game = [bool]$path -and ((Test-GamePath $path) -or $known -contains $path)
        $k = $stem.ToLower()
        if (-not $found.ContainsKey($k)) {
            $found[$k] = [pscustomobject]@{ Name = $(if ($name) { $name } else { $stem }); Exe = $stem; Game = $game }
        } elseif ($game) { $found[$k].Game = $true }
    }
    foreach ($path in $known) {
        # "...\steamapps\common\Portal 2\portal2.exe" -> "Portal 2"
        $title = if ($path -match '(?i)\\common\\([^\\]+)\\') { $Matches[1] } else { [IO.Path]::GetFileNameWithoutExtension($path) }
        & $add $path $title $path
    }
    $shell = New-Object -ComObject WScript.Shell
    $menus = "$env:ProgramData\Microsoft\Windows\Start Menu\Programs", "$env:APPDATA\Microsoft\Windows\Start Menu\Programs"
    foreach ($lnk in Get-ChildItem $menus -Recurse -Filter *.lnk -ErrorAction SilentlyContinue) {
        if ($lnk.BaseName -match 'uninstall|readme|help|website|documentation|release notes') { continue }
        $target = try { $shell.CreateShortcut($lnk.FullName).TargetPath } catch { '' }
        if ($target -like '*.exe') { & $add $target $lnk.BaseName $target }
    }
    foreach ($hive in 'HKLM', 'HKCU') {
        Get-ChildItem "${hive}:\Software\Microsoft\Windows\CurrentVersion\App Paths" -ErrorAction SilentlyContinue |
            ForEach-Object { & $add $_.PSChildName $null $null }
    }
    foreach ($p in Get-Process | Where-Object MainWindowHandle -ne 0) {
        $desc = try { $p.MainModule.FileVersionInfo.FileDescription } catch { $null }
        & $add $p.ProcessName $desc $p.Path
    }
    @($found.Values)
}

function Get-NormalizedSite($text) {
    ("$text".Trim().ToLower() -replace '^https?://', '' -replace '^www\.', '').TrimEnd('/')
}

# Full-screen picker: "Apps" tab (alphabetical, searchable, check to skip) and "Websites" tab.
function Edit-Exceptions {
    [Console]::CursorVisible = $false
    Clear-Host
    Write-Host "`n  $($C.Dim)Looking for your apps…$($C.Reset)"
    $s = Read-Settings
    $newSet = { $h = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase); foreach ($x in "$($args[0])" -split ',' | Where-Object { $_ }) { [void]$h.Add($x) }; , $h }
    $checked = & $newSet $s.ignored_apps      # apps where it's off
    $allowed = & $newSet $s.allowed_games     # games where it's on anyway
    $blockGames = $s.block_games -ne '0'
    $catalog = [System.Collections.Generic.List[object]]@(Get-AppCatalog)
    foreach ($x in @($checked) + @($allowed)) {
        if (-not ($catalog | Where-Object Exe -eq $x)) { $catalog.Add([pscustomobject]@{ Name = $x; Exe = $x; Game = $allowed.Contains($x) }) }
    }
    # games follow the allow list (off unless allowed); other apps follow the ignore list
    $isGame = { param($app) $blockGames -and $app.Game }
    $isOff = { param($app) if (& $isGame $app) { -not $allowed.Contains($app.Exe) } else { $checked.Contains($app.Exe) } }
    $catalog = @($catalog | Sort-Object Name)
    $sites = [System.Collections.Generic.List[string]]@($s.ignored_sites -split ',' | Where-Object { $_ })

    $tab = 0; $query = ''; $siteText = ''; $sel = 0; $top = 0; $done = $false
    Clear-Host
    while (-not $done) {
        $rows = [math]::Max(5, [Console]::WindowHeight - 11)
        if ($tab -eq 0) {
            $list = @(if ($query) { $catalog | Where-Object { $_.Name -like "*$query*" -or $_.Exe -like "*$query*" } } else { $catalog })
            $canAdd = [bool]$query -and -not ($catalog | Where-Object Exe -eq $query)
            $count = $list.Count + [int]$canAdd
        } else {
            $count = $sites.Count
        }
        $sel = [math]::Max(0, [math]::Min($sel, $count - 1))
        if ($sel -lt $top) { $top = $sel }
        if ($sel -ge $top + $rows) { $top = $sel - $rows + 1 }

        $out = [System.Text.StringBuilder]::new()
        $line = { param($t) [void]$out.Append("$t$E[K`n") }
        $tabNames = 'Apps', 'Websites'
        $tabs = for ($i = 0; $i -lt 2; $i++) {
            if ($i -eq $tab) { "$($C.Sel)$($C.Amber)$($C.Bold) $($tabNames[$i]) $($C.Reset)" } else { "$($C.Dim) $($tabNames[$i]) $($C.Reset)" }
        }
        & $line ''
        & $line "  $($C.Bold)Where Don't Forget works$($C.Reset)   $($tabs -join ' ')   $($C.Dim)Tab switches$($C.Reset)"
        & $line "  $($C.Dim)$('─' * 70)$($C.Reset)"
        if ($tab -eq 0) {
            $offCount = @($catalog | Where-Object { & $isOff $_ }).Count
            & $line "  Search: $($C.White)$query$($C.Amber)▏$($C.Reset)   $($C.Dim)$($list.Count) of $($catalog.Count) apps · $offCount off$(if ($blockGames) { ' · games are off unless switched on' })$($C.Reset)"
        } else {
            & $line "  Add a site: $($C.White)$siteText$($C.Amber)▏$($C.Reset)   $($C.Dim)e.g. youtube.com or github.com/settings$($C.Reset)"
        }
        & $line ''
        for ($r = $top; $r -lt [math]::Min($count, $top + $rows); $r++) {
            $on = $r -eq $sel
            $cursor = if ($on) { "$($C.Sel)$($C.Amber) › " } else { '   ' }
            $bg = if ($on) { $C.Sel } else { '' }
            if ($tab -eq 1) {
                $text = "$($C.Blue)◆$($C.Reset)$bg $($C.White)$($sites[$r])"
            } elseif ($r -lt $list.Count) {
                $app = $list[$r]
                $mark = if (& $isOff $app) { "$($C.Dim)○ off" } else { "$($C.Green)● on " }
                $name = if ($app.Name.Length -gt 40) { $app.Name.Substring(0, 39) + '…' } else { $app.Name.PadRight(40) }
                $tag = if ($app.Game) { "$($C.Amber)game  " } else { '      ' }
                $text = "$mark$($C.Reset)$bg $($C.White)$name $tag$($C.Dim)$($app.Exe)"
            } else {
                $text = "$($C.Amber)+$($C.Reset)$bg $($C.White)Add `"$query`" as a process name"
            }
            & $line "  $cursor$text $($C.Reset)"
        }
        if ($count -eq 0) {
            $empty = if ($tab -eq 0) { 'No apps match.' } else { 'No websites yet. Type one above and press Enter.' }
            & $line "     $($C.Dim)$empty$($C.Reset)"
        }
        & $line ''
        $help = if ($tab -eq 0) { 'type to search · ↑↓ move · Space/Enter switch on/off · Tab websites · Esc save' }
                else { 'type a site, Enter adds · ↑↓ move · Delete removes · Tab apps · Esc save' }
        & $line "  $($C.Dim)$help$($C.Reset)"
        [void]$out.Append("$E[J")
        [Console]::SetCursorPosition(0, 0)
        [Console]::Write($out.ToString())

        $k = [Console]::ReadKey($true)
        $key = "$($k.Key)"
        if ($key -eq 'Escape') { $done = $true }
        elseif ($key -eq 'Tab') { $tab = 1 - $tab; $sel = 0; $top = 0 }
        elseif ($key -eq 'UpArrow') { $sel-- }
        elseif ($key -eq 'DownArrow') { $sel++ }
        elseif ($key -eq 'PageUp') { $sel -= $rows }
        elseif ($key -eq 'PageDown') { $sel += $rows }
        elseif ($key -eq 'Home') { $sel = 0 }
        elseif ($key -eq 'End') { $sel = $count - 1 }
        elseif ($key -eq 'Backspace') {
            if ($tab -eq 0 -and $query) { $query = $query.Substring(0, $query.Length - 1); $sel = 0; $top = 0 }
            elseif ($tab -eq 1 -and $siteText) { $siteText = $siteText.Substring(0, $siteText.Length - 1) }
        }
        elseif ($key -eq 'Delete') { if ($tab -eq 1 -and $count -gt 0) { $sites.RemoveAt($sel) } }
        elseif ($tab -eq 0 -and $key -in 'Enter', 'Spacebar') {
            if ($sel -lt $list.Count) {
                $app = $list[$sel]
                $set = if (& $isGame $app) { $allowed } else { $checked }
                if (-not $set.Remove($app.Exe)) { [void]$set.Add($app.Exe) }
            } elseif ($canAdd) {
                $catalog = @(@($catalog) + [pscustomobject]@{ Name = $query; Exe = $query; Game = $false } | Sort-Object Name)
                [void]$checked.Add($query)
            }
        }
        elseif ($tab -eq 1 -and $key -eq 'Enter') {
            $site = Get-NormalizedSite $siteText
            if ($site -and -not $sites.Contains($site)) { $sites.Add($site) }
            $siteText = ''
        }
        else {
            $ch = $k.KeyChar
            if ([char]::IsLetterOrDigit($ch) -or "$ch" -match '[.\-_/:]') {
                if ($tab -eq 0) { $query += $ch; $sel = 0; $top = 0 } else { $siteText += $ch }
            }
        }
    }
    $s = Read-Settings
    $s.ignored_apps = (@($checked) | Sort-Object) -join ','
    $s.allowed_games = (@($allowed) | Sort-Object) -join ','
    $s.ignored_sites = $sites -join ','
    Save-Settings $s
    $msg = "Saved: $($checked.Count) app$(if ($checked.Count -ne 1) { 's' }) off, $($allowed.Count) game$(if ($allowed.Count -ne 1) { 's' }) on, $($sites.Count) website$(if ($sites.Count -ne 1) { 's' }) off."
    if (Get-AppProcess) { Say "$msg Restart the app to apply." $C.Amber } else { Say $msg }
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
    @{ Key = 'G'; Label = { param($s)
            $text = if ($s.Settings.block_games -ne '0') { 'off unless switched on per game' } else { 'on in games too' }
            "Games                $($C.Blue)‹ $text ›$($C.Reset)" }
        Run = { Switch-Games }; Step = { Switch-Games } }
    @{ Key = 'I'; Label = { param($s)
            $a = @($s.Settings.ignored_apps -split ',' | Where-Object { $_ }).Count
            $w = @($s.Settings.ignored_sites -split ',' | Where-Object { $_ }).Count
            "Apps & sites to skip $($C.Dim)$a app$(if ($a -ne 1) { 's' }) · $w site$(if ($w -ne 1) { 's' })$($C.Reset)" }; Run = { Edit-Exceptions } }
    @{ Key = 'E'; Label = { 'Open settings file' }; Run = { Start-Process notepad.exe $SettingsPath; Say 'Opened in Notepad.' } }
    @{ Key = 'B'; Label = { 'Rebuild from source' }; Run = { Build-App } }
    @{ Key = 'O'; Label = { 'Open GitHub page' }; Run = { Start-Process $Repo; Say 'Opened in your browser.' } }
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
    "  start with Windows: $(if ($s.Startup) { 'on' } else { 'off' }) · after a snip: $($AfterModes[$s.Settings.after_capture]) · hold: $($s.Settings.hold_ms) ms"
    "  skipped apps: $($s.Settings.ignored_apps) · skipped sites: $(if ($s.Settings.ignored_sites) { $s.Settings.ignored_sites } else { 'none' })"
    "  games: $(if ($s.Settings.block_games -ne '0') { 'off unless allowed' } else { 'on' }) · allowed games: $(if ($s.Settings.allowed_games) { $s.Settings.allowed_games } else { 'none' })"
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
