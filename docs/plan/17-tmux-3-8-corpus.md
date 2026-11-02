# 17 — The oracle against tmux 3.8

The compatibility oracle (`crates/tmxr-compat-oracle`) is measured against tmux
3.6: CI's `oracle` job builds the release named by `TMUX_VERSION` in
`.github/workflows/ci.yml`. tmux 3.8 is the latest release (2026-09-09), and the
owner asked (2026-11-01) for the corpus to follow the latest tmux, since 3.7 and
3.8 changed behaviour the corpus pins.

## Where it stands

The whole corpus was run on 2026-11-01 against Arch's tmux 3.8 (`tmux 3.8-1`)
with `TMUX_BIN=/usr/sbin/tmux`, `--test-threads=1`. These cases diverged
(_errors_: the tmux side refused the case's own commands):

- `formats.toml`: `t_modifier_and_strftime_in_display_message_are_tmuxs`,
  `e_t_q_and_m_chain_with_other_modifiers`,
  `format_loops_go_over_sessions_windows_and_panes`,
  `formats_read_option_values_as_tmux`,
  `loops_and_n_whose_lookups_fail_as_tmux`, `mode_formats_as_tmux`,
  `strftime_week_numbers_as_tmux`, `command_is_the_running_command_as_tmux`.
- `keys.toml`: `list_keys_in_tmuxs_order_and_layout`,
  `scrolling_keeps_the_cursors_row_and_history_bottom_its_end_as_tmux`,
  `a_searchs_formats_as_tmux`,
  `emacs_keys_clear_a_searchs_marks_on_a_move_as_tmux`,
  `copied_text_under_vi_keys_as_tmux`,
  `line_and_word_selections_move_by_lines_as_tmux`,
  `the_top_lines_time_as_tmux`, `list_keys_prints_commands_as_tmux`,
  `select_line_cursor_moves_as_tmux`,
  `select_word_on_a_line_of_wide_characters_as_tmux`,
  `a_binds_variables_print_expanded_as_tmux`, `blocks_print_in_braces_as_tmux`,
  `line_selection_back_where_it_was_made_as_tmux`; errors in
  `command_prompt_tab_completes_as_tmux`,
  `prompt_tab_session_menu_in_a_target_prompt_as_tmux` and
  `prompt_tab_window_menu_in_a_window_target_prompt_as_tmux` (3.8 has no
  `command-prompt -T target` / `window-target`).
- `paste.toml`: every `copy_cursor_x_follows_tmux_*` case and
  `copy_mode_rectangle_is_cleared_by_select_line`.
- `layout.toml`: `window_layout_is_tmuxs_string_and_select_layout_takes_it`
  (3.8's `#{window_layout}` is JSON),
  `split_window_with_an_empty_command_makes_an_empty_pane`.
- `options.toml`: `assume_paste_time_pastes_the_third_fast_key`,
  `status_takes_on_off_and_2_to_5_as_tmux`,
  `show_hooks_lists_hooks_in_tmuxs_order`.
- `mouse.toml`: `a_slider_drag_ends_on_the_slider`,
  `a_key_after_a_click_counts_toward_a_paste_as_tmux`.

Not in the corpus, found by review (2026-11-02, _reproduced_): 3.8's
`key-bindings.c` dropped `#{alternate_on}` from `MouseDrag1Pane`'s test, so a
drag in `less` or `man` copies text in tmux 3.8; tmxr's `defaults.toml` keeps
3.6's test and the drag does nothing.

Also seen by the slice-2 reviewer (2026-11-02): a click on an open menu's item
closes the menu in tmux 3.8 where tmxr's menu code (from 3.6) does not, with the
mouse on or off.

Each was only seen to diverge; none was read against tmux 3.7/3.8's source or
changelog yet, so which are tmux changes and which are tmxr bugs the 3.6 run hid
is not known.

## The work

1. Per diverging case, read the tmux change behind it (tmux's `CHANGES` for 3.7
   to 3.8) and decide: follow 3.8 (change tmxr and the case), or keep tmxr's
   behaviour and move the case to `known_divergences.toml` with the reason.
   Removed tmux features (the prompt types) are the owner's call: tmxr keeping
   them is a divergence, not a bug.
2. Set `TMUX_VERSION: '3.8'` in the `oracle` job and "measured against tmux 3.8"
   in `crates/tmxr-compat-oracle/README.md`, in the same change as the last
   case, so CI goes from green on 3.6 to green on 3.8.
3. Comments citing "tmux 3.6" behaviour stay true of 3.6; reword only those
   whose behaviour changed.
