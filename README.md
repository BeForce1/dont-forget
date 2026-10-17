# Don't Forget

Pin a piece of the screen next to your mouse so you don't lose track of the details while you work.

- **Hold the mouse wheel and drag** to snip part of the screen. The snip becomes a floating note.
- After a snip, the note **follows your mouse**. You can change this in the tray menu: it can follow until your next click and then stay there, or stay where you snipped it.
- **Drag right→left** to add the new note to the ones already following; they stack next to the cursor. Dragging left→right leaves the old note where it is, and only the new one follows.
- **Wheel-click anywhere** to drop the following notes where they are.
- **Hold Shift** to freeze following notes so you can reach them with the mouse.
- **Wheel-click a note** to delete it, or hover over it and click the ×.
- **Click a note** for options: follow or pin, opacity, size, copy, delete. **Drag** a note to move it, and **scroll** on it to resize it.
- When no notes are following you, a quick wheel click works normally in other apps (opening links in new tabs, closing tabs, and so on).
- Apps where the wheel is used for dragging, such as CAD programs, are left alone. Edit this list from the tray menu under *Ignored apps…*.

Windows 10/11 only. It's a single ~400 KB `.exe` with no runtime to install, using about 2.5 MB of RAM and no CPU at idle.

To build it: `cargo build --release` (Rust; the `x86_64-pc-windows-gnu` toolchain works without Visual Studio).
Settings live in `%APPDATA%\DontForget\settings.ini`.
