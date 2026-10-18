//! tmux's session options: one typed table, global (`set -g`) with what each
//! session sets for itself over it, read by `set-option` and printed by
//! `show-options` under tmux's names.

use crate::table::{Array, Word, options_table};
use crate::{AlertAction, DestroyUnattached, Visual};

options_table! {
    /// The session options every session has unless it sets its own: tmux's
    /// global session options (`set -g`).
    table SessionOptions;
    /// The options one session sets itself (`set` without `-g`), over the
    /// global ones, and its own `@name` user options.
    own OwnSessionOptions;
    /// Which windows' activity alerts act, relative to the current one.
    "activity-action" => activity_action: AlertAction,
    /// The index a new session's first window gets, and the first free
    /// one looked for.
    "base-index" => base_index: u32,
    "bell-action" => bell_action: AlertAction,
    /// What a new pane runs when given no command, by the shell; empty,
    /// the shell itself.
    "default-command" => default_command: String,
    /// The shell new panes run; none (or empty) is the platform's default,
    /// as tmxr-term's `pty::default_shell` picks it.
    "default-shell" => default_shell: Option<String>,
    /// Whether the session goes once no client is attached to it.
    "destroy-unattached" => destroy_unattached: DestroyUnattached,
    /// `display-panes` number colour for the active pane, and the others.
    "display-panes-active-colour" => display_panes_active_colour: Word,
    "display-panes-colour" => display_panes_colour: Word,
    /// How long `display-panes` shows pane numbers, in milliseconds.
    "display-panes-time" => display_panes_time: u64,
    /// How long a status message shows, in milliseconds.
    "display-time" => display_time: u64,
    /// Lines of history a new pane keeps.
    "history-limit" => history_limit: usize,
    /// The key table a client's keys start in (tmux's `root` by default).
    "key-table" => key_table: String,
    /// Seconds a client may sit idle before it is locked; 0 never.
    "lock-after-time" => lock_after_time: u64,
    /// Shell command a locked client runs in its terminal; the client is
    /// unlocked when it exits.
    "lock-command" => lock_command: String,
    "message-style" => message_style: String,
    "mouse" => mouse: bool,
    /// The prefix key, by tmux key name.
    "prefix" => prefix: Word,
    /// A second prefix key; `None` for none.
    "prefix2" => prefix2: Word,
    "renumber-windows" => renumber_windows: bool,
    /// How long a repeatable bind (`bind -r`) takes another key without
    /// the prefix, in milliseconds.
    "repeat-time" => repeat_time: u64,
    /// Whether clients' terminals get a title, as `set-titles-string`
    /// makes it for the active pane.
    "set-titles" => set_titles: bool,
    "set-titles-string" => set_titles_string: String,
    "silence-action" => silence_action: AlertAction,
    /// Seconds between status line redraws.
    "status-interval" => status_interval: u64,
    "status-left" => status_left: String,
    /// Most cells `status-left` may take.
    "status-left-length" => status_left_length: u16,
    /// The style `status-left` is drawn over, on the status line's.
    "status-left-style" => status_left_style: String,
    "status-right" => status_right: String,
    /// Most cells `status-right` may take; a longer right side is cut at
    /// its end so the window list keeps its room.
    "status-right-length" => status_right_length: u16,
    /// The style `status-right` is drawn over, on the status line's.
    "status-right-style" => status_right_style: String,
    "status-style" => status_style: String,
    /// The client environment variables a session takes when it is made or
    /// attached: `fnmatch` patterns, as tmux's.
    "update-environment" => update_environment: Array,
    /// How alerts act: the terminal's bell (`off`), a status message (`on`)
    /// or both.
    "visual-activity" => visual_activity: Visual,
    "visual-bell" => visual_bell: Visual,
    "visual-silence" => visual_silence: Visual,
    /// The characters that end a word in copy mode, besides blanks.
    "word-separators" => word_separators: String,
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;

    #[test]
    fn every_name_is_shown_in_tmux_order() {
        let o = crate::defaults().session;
        for n in SessionOptions::NAMES {
            assert!(o.show(n).is_some(), "{n}");
        }
        let mut sorted = SessionOptions::NAMES.to_vec();
        sorted.sort_unstable();
        assert_eq!(SessionOptions::NAMES, sorted);
    }

    #[test]
    fn a_sessions_own_options_override_the_global_ones() {
        let global = crate::defaults().session;
        let mut own = OwnSessionOptions::default();
        own.set("status-left", Some("x"), false, &global).unwrap();
        own.set("status-left", Some("y"), true, &global).unwrap();
        own.set("mouse", None, false, &global).unwrap();
        own.set("bell-action", Some("none"), false, &global)
            .unwrap();
        let o = own.over(Cow::Borrowed(&global));
        assert_eq!(o.status_left, "xy");
        assert_eq!(o.mouse, !global.mouse);
        assert_eq!(o.bell_action, AlertAction::None);
        assert_eq!(o.prefix, global.prefix);
        assert_eq!(
            own.set("visual-bell", Some("x"), false, &global)
                .unwrap_err(),
            "visual-bell: unknown value: x (off, on or both)"
        );
        assert_eq!(
            own.set("status-left-length", Some("70000"), false, &global)
                .unwrap_err(),
            "status-left-length: bad number 70000"
        );
        own.unset("status-left");
        assert_eq!(own.show("status-left"), None);
        assert_eq!(
            own.over(Cow::Borrowed(&global)).status_left,
            global.status_left
        );
    }

    #[test]
    fn no_option_is_in_both_tables() {
        for n in SessionOptions::NAMES {
            assert!(!crate::WindowOptions::NAMES.contains(n), "{n}");
        }
    }
}
