# Third-party notices

## gpui-terminal (adapted)

Portions of `src/terminal/gpui_emu/` are adapted from
[gpui-terminal](https://github.com/zortax/gpui-terminal) v0.1.0
(MIT OR Apache-2.0), Copyright (c) Leonard Seibold.

Loom retains the MIT option and adds `Event::PtyWrite` writeback to the PTY
so shells that query cursor position (e.g. PowerShell CSI `6 n`) do not stall.
