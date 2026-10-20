# Don't Forget

**A screenshot that follows your mouse, so you never forget the details.** Hold the mouse wheel and drag over part of the screen. That piece stays next to your cursor while you work.

![Snipping a problem, carrying it to the edge of the screen, and pinning it there while reading the solution](docs/pin.gif)

## How to use it

![Green left-to-right and blue right-to-left snips](docs/modes.gif)

| Do this | What happens |
|---|---|
| Hold the wheel and drag **left → right** | Snip. The note (green) follows your mouse. |
| Hold the wheel and drag **right → left** | Snip and add it (blue) to the notes already following you. |
| Quick wheel click anywhere | Drop the following notes where they are. |
| Wheel click on a note | Delete it. |
| Hold **Shift** | Freeze the following notes so you can reach them. |
| Click a note | Options: follow or pin, opacity, size, copy, delete. You can also drag it to move it and scroll on it to resize it. |

**Games are left alone.** In games (anything Windows recognizes as a game, anything installed in a Steam, Epic, GOG, Xbox, Riot, Ubisoft, EA or Rockstar library, or a full-screen DirectX game) your mouse wheel works exactly as the game expects. You can switch Don't Forget back on for a specific game in the control panel.

A quick wheel click still works normally in other apps when no notes are following you. Apps that use wheel-drag themselves, like SolidWorks and Blender, are left alone. You can edit that list from the tray icon under *Ignored apps…*.

**Websites can be skipped too** (Windows). Add sites like `youtube.com` in the control panel and the wheel works normally on them in Chrome, Edge, Firefox, Brave, Opera, Vivaldi, Arc, Zen, LibreWolf, Waterfox and Floorp.

## Download

Grab the latest build from [Releases](https://github.com/BeForce1/dont-forget/releases). It's one codebase with one native binary per OS:

- **Windows:** `DontForget-windows-x86_64.exe`. Run it and it sits in the tray.
- **Linux:** `DontForget-linux-x86_64`. Run `chmod +x` on it, then start it inside your X session.

## Control panel (Windows)

Double-click `control.cmd` (needs PowerShell 7) to start, stop or restart the app, toggle start with Windows, choose apps, games and websites to skip, change settings, or rebuild it. `control.cmd -Status` prints the current state.

## Linux

It runs in the background of your X session and is controlled from the terminal:

```
dont-forget            start
dont-forget --status   is it running, how many notes
dont-forget --clear    delete all notes
dont-forget --reload   re-read the settings file
dont-forget --quit     stop it
```

Games are detected by their library folders, including Steam games running through Proton. Website skipping is Windows-only for now. Settings are stored in `~/.config/dont-forget/settings.ini`, with the same keys as on Windows.

## Build

Run `cargo build --release` with Rust. On Windows, the `x86_64-pc-windows-gnu` toolchain works without Visual Studio, and settings are stored in `%APPDATA%\DontForget\settings.ini`. The Linux build needs no system libraries beyond glibc, because it talks to X11 directly. `tools/linux-build.sh` builds it inside WSL.

## Requirements

| | Windows | Linux |
|---|---|---|
| **System** | Windows 10 (1703 or later) or 11, 64-bit. On Windows 10, notes have square corners. | x86-64 with glibc 2.35+ (Ubuntu 22.04+, Debian 12+, Fedora 36+), an **X11** session with the XTEST and SHAPE extensions (standard on Xorg), and 24 or 32-bit color. Wayland isn't supported yet. |
| **Mouse** | Any mouse with a clickable scroll wheel (middle button). | Same. |
| **CPU** | Any 64-bit processor. 0% while idle. | Same, 0% while idle. |
| **Memory** | About 2 MB idle (1.9 MB private, 1 MB in Task Manager), or about 3 MB with a websites list, which loads UI Automation. | About 3 MB resident idle. |
| **Disk** | One 444 KB `.exe`, with nothing to install. | One 574 KB binary. |
| **Extras** | PowerShell 7 for the optional control panel. | None. |

Each note holds its snip as an uncompressed image, so it adds roughly width × height × 4 bytes. An 800×500 snip adds about 1.6 MB.

<details><summary>How these were measured</summary>

Release build v0.2.0 on a 20-thread Intel laptop running Windows 11. Startup takes about 12 ms until the message loop is ready (84 ms on a cold first launch). The app was then sampled idle for 30 s with no notes open: 0 ms of CPU time without a websites list, and 15.6 ms (0.003%) with one. Linux was measured under Xvfb in WSL2 (Ubuntu): 3.1 MB resident (192 KB anonymous), 0 CPU ticks in 30 s, and roughly 0.2 s from launch until the control socket answers.
</details>
