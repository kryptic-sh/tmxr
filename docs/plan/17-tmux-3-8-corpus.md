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
3. **vi copy mode keeps the cursor off the cell past the text**
   (`window_copy_update_cursor`, 3.7): the cursor is clamped to the line's
   visible end (and off wide characters' padding), which changes the remembered
   column, line copies and a rectangle's edge. Cases: every
   `copy_cursor_x_follows_tmux_*`, `select_line_cursor_moves_as_tmux`,
   `select_word_on_a_line_of_wide_characters_as_tmux`,
   `line_selection_back_where_it_was_made_as_tmux`,
   `line_and_word_selections_move_by_lines_as_tmux`,
   `copied_text_under_vi_keys_as_tmux`,
   `copy_mode_rectangle_is_cleared_by_select_line` (this one's link to the clamp
   is a guess). Medium (`copy/`).
4. **vi scrolling keeps the cursor on its text** (3.8, issue 5216):
   `scrolling_keeps_the_cursors_row_and_history_bottom_its_end_as_tmux`. Small.
5. **Copy mode stays at the bottom while selecting** (3.8, issue 5349):
   `copy_mode_e_leaves_at_the_bottom_as_tmux`. Small.
6. **A cleared search drops its counts** (`window_copy_clear_marks`, 3.7):
   `a_searchs_formats_as_tmux`,
   `emacs_keys_clear_a_searchs_marks_on_a_move_as_tmux`. Small.
7. **No paste guessing when the terminal brackets pastes**
   (`server_client_is_assume_paste`, 3.7), which is every VT100-like one:
   `assume_paste_time_pastes_the_third_fast_key`,
   `a_key_after_a_click_counts_toward_a_paste_as_tmux`. Small.
8. **New default status lines 2 to 5** (`options-table.c`: the
   `window-pane-status-format` options, `pane-status-current-style`
   `underscore`): `status_takes_on_off_and_2_to_5_as_tmux`,
   `status_3_draws_the_pane_and_session_lines_as_tmux`,
   `message_line_puts_a_message_on_that_status_line_as_tmux`,
   `formats_read_option_values_as_tmux`. Small to medium.
9. **Themes** (3.8): mode formats gain theme colours, and `clock-mode-colour` is
   a string option defaulting to `themeblue`, so `show-options` quotes it:
   `mode_formats_as_tmux`, `show_options_quotes_strings_as_tmux`. The defaults
   are small; themes themselves are large.
10. **New hooks** (3.8): `show_hooks_lists_hooks_in_tmuxs_order` (and the known
    divergence `show_hooks_g_lists_the_hooks_tmxr_fires`). Listing them is
    small; firing them is large.
11. **A hook's `#{command}` is the command that fired it**
    (`hooks_insert_event`): `command_is_the_running_command_as_tmux`. Medium.
12. **`q:` also escapes braces** (`format_quote_shell`):
    `e_t_q_and_m_chain_with_other_modifiers`. Small.
13. **Loops**: `loop_last_flag` is unset outside a loop, and `#{L:}` counts only
    attached clients (`sort_get_clients`):
    `format_loops_go_over_sessions_windows_and_panes`,
    `loops_and_n_whose_lookups_fail_as_tmux`. Small.
14. **JSON layout strings** (3.8; the old form still accepted):
    `window_layout_is_tmuxs_string_and_select_layout_takes_it`. Medium.
15. **An empty pane has no pid and is not dead**:
    `split_window_with_an_empty_command_makes_an_empty_pane`. Small.
16. **Scrollbars no longer narrow the pane** (`modal` is an overlay, 3.8
    auto-hide): `scrollbars_modal_in_copy_mode`. Medium.
17. **More terminals get extended keys** (3.8 names WezTerm, ghostty and Rio
    from XTVERSION, each with `extkeys`):
    `a_terminal_named_as_neither_is_not_asked_for_modify_other_keys`. Small; the
    case needs another terminal name.
18. **Unknown**: `a_slider_drag_ends_on_the_slider` (tmxr 152, 3.8 159). Bisect
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
