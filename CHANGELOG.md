# Changelog

## Unreleased

**Linux catches up with Windows**
- **Notes are kept** on Linux too, as PNGs in `~/.config/dont-forget/notes`, and come back where they were. Notes that were saved off-screen move back onto the screen, also when the screen size changes.
- **Undo a delete:** **Ctrl+Alt+Z** or `dont-forget --restore`. `--clear` counts as one step, and the last 20 are kept.
- **Keyboard shortcuts:** **Ctrl+Alt+S** to snip with a left-button drag (Esc or a right click cancels), and **Ctrl+Alt+H** to hide or show all notes. The keys can be changed in the settings file, and if your desktop already uses one, the app tells you when it starts. There are new `--snip` and `--hide` commands too, for binding in your desktop's keyboard settings.
- **Every new snip is copied to the clipboard** as a PNG, ready to paste (`copy_snips`). Large snips are sent in pieces.
- **The look settings work on Linux:** colors, solid, dashed or dotted frames, and frame thickness. `--reload` applies them to the notes already open.
- Skipping websites is still Windows-only.

## 0.3.0 (2026-10-23)

**Download:** `DontForget-windows-x86_64.exe` for Windows 10/11 (x64), or `DontForget-linux-x86_64` for Linux on X11 (x64, glibc 2.35+). One file each, no install. Windows may warn that the app is from an unknown publisher, because it isn't code-signed. Click *More info*, then *Run anyway*.

**New**
- **Notes are kept.** Your notes come back where you left them after a restart, a sign-out or a crash. They're stored as PNGs in `%APPDATA%\DontForget\notes`. Turn this off with *Keep notes after restart* in the tray menu, and the folder is emptied when the app quits.
- **Undo a delete.** **Ctrl+Alt+Z** (or *Restore deleted note* in the tray menu) brings back the last deleted note. After *Delete all notes*, it brings back all of them. The last 20 are kept.
- **Snip with the keyboard.** Press **Ctrl+Alt+S**, then drag with the left button. This works on touchpads and in apps where wheel-drag is turned off.
- **Hide all notes.** **Ctrl+Alt+H** hides every note without deleting it (when sharing your screen, say), and shows them again.
- **Pick your look.** New default colors (amber for a snip, violet for one added to the stack), three more color sets, solid, dashed or dotted frames, and thin, normal or thick frames. All of it is in the tray menu under *Look*, and any colors can be typed into the settings file.
- All three shortcuts can be changed or turned off in the settings file.
- The control panel's changes now apply right away, without a restart, and **Stop** keeps your notes.
- The tray menu has *Snip now*, *Hide notes*, *Restore deleted note*, *Look*, *Keep notes after restart* and *About*.
- The Windows exe has its own icon and version details.

**Fixes**
- Notes that end up off-screen, for example after unplugging a monitor, move back onto a screen you have.
- With monitors at different scaling, the snip outline, size label and note frames are now sized for the monitor they're on.
- A new note no longer shifts a pixel or two to the side while it pops in.

## 0.2.0 (2026-10-20)

- Linux (X11) version built from the same code.
- Off in games unless you switch it on for a game. Websites can be skipped too.
- Every new snip is also copied to the clipboard.
- Fixes: stale app detection when Windows reused a process id, a clipboard memory leak, a control panel crash on a mistyped `hold_ms`, and settings that a crash mid-save could wipe.

## 0.1.0 (2026-10-17)

- First release: wheel-drag snips that float next to your mouse, in one small Windows exe.
