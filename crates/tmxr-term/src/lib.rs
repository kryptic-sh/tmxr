//! One tmxr pane's terminal.
//!
//! Will own PTY spawning (`portable-pty`), the per-pane `vt100` emulator,
//! key and mouse encoding for the child, and foreground-process / working
//! directory inspection on every platform
//! (`docs/plan/04-panes-and-terminal.md`).
//!
//! Empty in the scaffold: nothing is implemented yet. Panes land in milestone
//! M1 (`docs/plan/15-milestones.md`).
