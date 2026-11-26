# 17 — The oracle against tmux 3.8

The compatibility oracle (`crates/tmxr-compat-oracle`) is measured against tmux
3.6: CI's `oracle` job builds the release named by `TMUX_VERSION` in
`.github/workflows/ci.yml`. tmux 3.8 is the latest release (2026-09-09), and the
owner asked (2026-11-01) for the corpus to follow the latest tmux, since 3.7 and
3.8 changed behaviour the corpus pins.

## Where it stands

The whole corpus was run on 2026-11-27 against Arch's tmux 3.8 (`TMUX_BIN` the
system tmux, `--test-threads=1`, tmxr at `135edbe`): 61 cases diverge, every one
because tmux 3.8 itself no longer does what the case recorded against 3.6 (each
`Unexpected` was on the tmux side). Each was traced to a 3.7 / 3.8 change by
reading tmux 3.8's `CHANGES` and source; none is a tmxr bug the 3.6 runs hid,
and none is a fragile case. What 3.6 did is inferred from the recorded values,
not read in 3.6's source. Grouped by cause:

2. **The command prompt's Tab completion** (`prompt.c`): 3.8 knows only the
   `command` and `search` prompt types (no `target` / `window-target`), and
   completes command names inline, underlined, with no menu and no session or
   window names. Cases: every `prompt_tab_*` and
   `command_prompt_tab_completes_as_tmux`. Following it removes tmxr's menu.
3. **New hooks** (3.8): `show_hooks_lists_hooks_in_tmuxs_order` (and the known
   divergence `show_hooks_g_lists_the_hooks_tmxr_fires`). Listing them is small;
   firing them is large.
4. **A hook's `#{command}` is the command that fired it**
   (`hooks_insert_event`): `command_is_the_running_command_as_tmux`. Medium.
5. **JSON layout strings** (3.8; the old form still accepted):
   `window_layout_is_tmuxs_string_and_select_layout_takes_it`. Medium.
6. **Scrollbars no longer narrow the pane** (`modal` is an overlay, 3.8
   auto-hide): `scrollbars_modal_in_copy_mode`. Medium.
7. **Unknown**: `a_slider_drag_ends_on_the_slider` (tmxr 152, 3.8 159). Bisect
   with the 3.7 / 3.7b / 3.7c packages in the pacman cache.

These cases in `known_divergences.toml` match 3.8 and move to their corpus files
when CI does: `decrqm_reports_modes_tmux_3_6_does_not`,
`decrqm_reports_synchronized_output_as_tmux_3_8`,
`a_synchronized_redraw_is_held_until_its_end`,
`mouse_motion_leaves_clock_mode_with_the_mouse_off`,
`presses_on_two_panes_are_two_clicks`.

Not in the corpus, found by review (2026-11-02, _reproduced_): 3.8's
`key-bindings.c` dropped `#{alternate_on}` from `MouseDrag1Pane`'s test, so a
drag in `less` or `man` copies text in tmux 3.8; and a click on an open menu's
item closes the menu in 3.8 where tmxr's (from 3.6) does not.

## The work

Decided by the owner (2026-11-27): follow tmux 3.8 in every group above but the
Tab completion menu (parked, see the backlog), including JSON layouts, with the
old layout string still read (`select-layout`, resurrect saves). New hooks are
listed as 3.8 lists them; firing each, and themes beyond their defaults, are
backlog work.

CI's `oracle` job builds tmux 3.8. Every case that diverged sits in
`known_divergences.toml`, marked `tmux 3.8: <cause> (from <file>)`; each change
that follows 3.8 for a group moves its cases back to the file named, so CI stays
green against 3.8 throughout. When the last group is done, this phase is
finished: what is left goes to the backlog and this file goes.
