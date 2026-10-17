# Don't Forget

**A screenshot that follows your mouse.** Hold the mouse wheel, drag over part of the screen, and that piece stays next to your cursor while you work.

![Snipping a problem, carrying it to the edge of the screen, and pinning it there while reading the solution](docs/pin.gif)

## Why I made this

In class I give Claude a problem and then read through its solution. Halfway down I've forgotten the problem itself: what was the set again, and what exactly did part (c) ask? So I scroll back up, lose my place, and scroll down again. I wanted the problem to come with me while I read, so I made this.

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

A quick wheel click still works normally in other apps when no notes are following you. Apps that use wheel-drag themselves, like SolidWorks and Blender, are left alone. You can edit that list from the tray icon under *Ignored apps…*.

## Install

Windows 10/11. It's a single ~400 KB `.exe` with nothing else to install, and it uses about 2.5 MB of RAM and no CPU while idle.

To build it, run `cargo build --release` with Rust. The `x86_64-pc-windows-gnu` toolchain works without Visual Studio. Settings are stored in `%APPDATA%\DontForget\settings.ini`.
