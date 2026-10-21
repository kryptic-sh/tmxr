//! Property tests of the command language, as plan 14's fuzzing asks: any
//! text tokenizes or fails cleanly, any format expands without a panic, and
//! the quoting functions give words that read back as they were.

use proptest::prelude::*;
use tmxr_command::format::{Context, LoopItem, LoopKind, NameKind, expand};
use tmxr_command::tokenize::{escape_arg, join_args, tokenize};

/// A format context that knows a few names, enough to reach the lookups.
struct Vars;

impl Context for Vars {
    fn get(&self, name: &str) -> Option<String> {
        matches!(name, "a" | "b" | "session_name").then(|| format!("<{name}>"))
    }

    fn name_exists(&self, _: NameKind, _: &str) -> Option<bool> {
        Some(false)
    }

    fn pane_lines(&self) -> Option<Vec<String>> {
        Some(vec!["one".into(), "two".into()])
    }

    fn loop_items(&self, _: LoopKind) -> Option<Vec<LoopItem<'_>>> {
        Some(Vec::new())
    }
}

fn no_env(_: &str) -> Option<String> {
    None
}

/// Words of any characters a terminal could type, the command language's
/// specials among them.
fn word() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[a-z0-9 ;'\"\\\\$#{}~%!,:=\t\n-]{0,12}|.{0,6}").unwrap()
}

proptest! {
    #[test]
    fn any_line_tokenizes_or_fails_cleanly(line in ".{0,64}") {
        let _ = tokenize(&line, &no_env);
    }

    #[test]
    fn any_format_expands(fmt in "([#{}:,;?=!<>|a-z0-9%()\\[\\]/ -]|#\\{|\\}){0,40}") {
        let _ = expand(&fmt, &Vars);
    }

    #[test]
    fn joined_words_read_back_as_one_command(words in proptest::collection::vec(word(), 1..6)) {
        prop_assert_eq!(tokenize(&join_args(&words), &no_env).unwrap(), vec![words]);
    }

    #[test]
    fn an_escaped_word_reads_back_as_itself(word in word()) {
        let escaped = escape_arg(&word);
        // An escape inside single quotes reads back as written, as tmux's.
        prop_assume!(!(escaped.starts_with('\'') && escaped.contains('\\')));
        prop_assert_eq!(tokenize(&escaped, &no_env).unwrap(), vec![vec![word]]);
    }
}
