//! Split a tmux command line into a command list.
//!
//! Follows tmux's quoting: words split on whitespace; `'…'` is literal;
//! outside it, in `"…"` or bare, a backslash escapes as tmux's
//! (`yylex_token_escape`: `\n`, `\t`, `\e` and the other C escapes, `\s` a
//! space, `\ooo` octal, `\uXXXX` / `\UXXXXXXXX`, else the character itself)
//! and `$VAR` / `${VAR}` expands; `~` at the start of a word is `$HOME`. A `;` ending a word (or standing alone)
//! separates commands; `\;` is a literal `;`. A word starting with `#` (other
//! than a `#{format}`) starts a comment. `{ … }` is one word holding the
//! commands inside as tmux prints them back (`crate::print_block`), as tmux
//! passes a block of commands to `if-shell`, `bind-key` and the like.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenizeError {
    #[error("unterminated {0} quote")]
    Unterminated(char),
    #[error("unterminated ${{ in variable reference")]
    UnterminatedBrace,
    #[error("unterminated {{ … }} block")]
    UnterminatedBlock,
    #[error("{0}")]
    Escape(&'static str),
}

/// Parse `line` into commands, each a list of words. `env` resolves
/// `$VAR` references (unset variables expand to nothing, as in a shell).
pub fn tokenize(
    line: &str,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<Vec<String>>, TokenizeError> {
    Ok(tokenize_lines(line, env)?
        .into_iter()
        .map(|c| c.words)
        .collect())
}

/// A command as [`tokenize_lines`] reads it: its words, and which of them
/// were a `{ … }` block, which tmux keeps as a command list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line {
    pub words: Vec<String>,
    /// Which of the words were a `{ … }` block.
    pub blocks: Vec<bool>,
    /// An empty command (`;;`) came before it: tmux's next command group,
    /// as a block's next line is.
    pub new_group: bool,
}

impl From<Vec<String>> for Line {
    /// Words with no block among them (an argv from a shell).
    fn from(words: Vec<String>) -> Self {
        let blocks = vec![false; words.len()];
        Self {
            words,
            blocks,
            new_group: false,
        }
    }
}

/// [`tokenize`], keeping which words were blocks and where command groups
/// start.
pub fn tokenize_lines(
    line: &str,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<Line>, TokenizeError> {
    let mut commands: Vec<Line> = Vec::new();
    let mut command = Line::default();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = line.chars().peekable();

    let end_word = |command: &mut Line, word: &mut String, in_word: &mut bool| {
        if *in_word {
            command.words.push(std::mem::take(word));
            command.blocks.push(false);
            *in_word = false;
        }
    };
    let end_command = |command: &mut Line, commands: &mut Vec<Line>| {
        if command.words.is_empty() {
            command.new_group |= !commands.is_empty();
        } else {
            commands.push(std::mem::take(command));
        }
    };

    while let Some(c) = chars.next() {
        match c {
            // Spaces and tabs part words; any other character, Unicode's
            // other spaces among them, is part of one, as in tmux's lexer.
            ' ' | '\t' => end_word(&mut command, &mut word, &mut in_word),
            // A line ends the command, as `;` does; `\r\n` is a line end.
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                end_word(&mut command, &mut word, &mut in_word);
                end_command(&mut command, &mut commands);
            }
            '#' if !in_word && chars.peek() != Some(&'{') => break,
            '{' if !in_word => {
                command.words.push(block(&mut chars)?);
                command.blocks.push(true);
            }
            ';' => {
                // `;` ends the current word and the command.
                end_word(&mut command, &mut word, &mut in_word);
                end_command(&mut command, &mut commands);
            }
            '\\' => {
                in_word = true;
                escape(&mut chars, &mut word)?;
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => word.push(ch),
                        None => return Err(TokenizeError::Unterminated('\'')),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') if chars.peek().is_none() => {
                            return Err(TokenizeError::Unterminated('"'));
                        }
                        Some('\\') => escape(&mut chars, &mut word)?,
                        Some('$') => expand_var(&mut chars, &mut word, env)?,
                        Some(ch) => word.push(ch),
                        None => return Err(TokenizeError::Unterminated('"')),
                    }
                }
            }
            '~' if !in_word => {
                in_word = true;
                let next_ends_word = chars
                    .peek()
                    .is_none_or(|n| matches!(n, '/' | ' ' | '\t' | '\n' | ';'));
                if next_ends_word {
                    word.push_str(&env("HOME").unwrap_or_else(|| "~".into()));
                } else {
                    word.push('~');
                }
            }
            '$' => {
                in_word = true;
                expand_var(&mut chars, &mut word, env)?;
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    end_word(&mut command, &mut word, &mut in_word);
    end_command(&mut command, &mut commands);
    Ok(commands)
}

/// Quote words so they survive another trip through the tokenizer.
pub fn join_args(words: &[String]) -> String {
    words
        .iter()
        .map(|w| {
            if !w.is_empty()
                && w.chars()
                    .all(|c| c.is_alphanumeric() || "-_./:%@#{}=+,^".contains(c))
                && !w.starts_with(['#', '{'])
            {
                w.clone()
            } else {
                format!("'{}'", w.replace('\'', r"'\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `word` quoted as tmux 3.6's `args_escape` quotes a value for
/// `show-options` (with the escapes of its `utf8_strvis` and `vis`): in
/// double quotes when it holds any of ` #';${}%`, else in single quotes when
/// it holds a `"`; a single such character, or a lone `~`, as `\c`; control
/// characters as C escapes or octal, and a leading `~` escaped. What it
/// gives reads back through [`tokenize`] as `word`, except where tmux's own
/// does not either: an escape inside single quotes (a doubled backslash,
/// a control character's) reads back as written, as tmux 3.6's does.
pub fn escape_arg(word: &str) -> String {
    const DOUBLE: &str = " #';${}%";
    const SINGLE: &str = " \"";
    if word.is_empty() {
        return "''".into();
    }
    let quote = if word.contains(|c| DOUBLE.contains(c)) {
        Some('"')
    } else if word.contains(|c| SINGLE.contains(c)) {
        Some('\'')
    } else {
        None
    };
    if word.len() == 1 && word != " " && (quote.is_some() || word == "~") {
        return format!("\\{word}");
    }
    let double = quote == Some('"');
    let mut escaped = String::with_capacity(word.len());
    let mut chars = word.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '$' if double
                && chars
                    .peek()
                    .is_some_and(|n| n.is_ascii_alphabetic() || matches!(n, '_' | '{')) =>
            {
                escaped.push_str("\\$");
            }
            '"' if double => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\x08' => escaped.push_str("\\b"),
            '\x07' => escaped.push_str("\\a"),
            '\x0b' => escaped.push_str("\\v"),
            '\t' => escaped.push_str("\\t"),
            '\x0c' => escaped.push_str("\\f"),
            // Where tmux's vis writes `\0` with no octal digit after, which
            // its own parser refuses; a tmux value never holds a NUL to show.
            '\0' => escaped.push_str("\\000"),
            c if c == ' ' || c.is_ascii_graphic() || !c.is_ascii() => escaped.push(c),
            c => {
                escaped.push('\\');
                for shift in [6, 3, 0] {
                    escaped.extend(char::from_digit(u32::from(c) >> shift & 7, 8));
                }
            }
        }
    }
    let tilde = if escaped.starts_with('~') { "\\" } else { "" };
    match quote {
        Some('\'') => format!("'{escaped}'"),
        Some(_) => format!("\"{tilde}{escaped}\""),
        None => format!("{tilde}{escaped}"),
    }
}

/// The commands in a `{ … }` block, its `{` already read, up to the
/// matching `}`: one per line or `;`, printed back as tmux stores a block
/// (`crate::print_block`), to be parsed again when they run. A block that
/// does not parse is kept as written, its lines joined by ` ;; `.
fn block(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Result<String, TokenizeError> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut depth = 0usize;
    let mut quote = None;
    loop {
        let c = chars.next().ok_or(TokenizeError::UnterminatedBlock)?;
        if let Some(q) = quote {
            line.push(c);
            if c == '\\' && q == '"' {
                line.extend(chars.next());
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                line.push(c);
            }
            '\\' => {
                line.push(c);
                line.extend(chars.next());
            }
            '{' => {
                depth += 1;
                line.push(c);
            }
            '}' if depth == 0 => break,
            '}' => {
                depth -= 1;
                line.push(c);
            }
            '\n' => lines.push(std::mem::take(&mut line)),
            c => line.push(c),
        }
    }
    lines.push(line);
    let raw = lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ;; ");
    Ok(crate::print_block(&raw).unwrap_or(raw))
}

/// The escape after a backslash, as tmux 3.6's `yylex_token_escape` reads
/// it. Octal escapes give bytes, so a run of them (`\303\251`) is read
/// together as UTF-8; bytes that are not valid UTF-8 become U+FFFD, where
/// tmux keeps them raw.
fn escape(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    word: &mut String,
) -> Result<(), TokenizeError> {
    let c = chars
        .next()
        .ok_or(TokenizeError::Escape("trailing backslash"))?;
    let c = match c {
        '0'..='7' => {
            let mut bytes = vec![octal(c, chars)?];
            while chars.peek() == Some(&'\\') {
                let Some(d @ '0'..='7') = chars.clone().nth(1) else {
                    break;
                };
                chars.next();
                chars.next();
                bytes.push(octal(d, chars)?);
            }
            word.push_str(&String::from_utf8_lossy(&bytes));
            return Ok(());
        }
        'a' => '\x07',
        'b' => '\x08',
        'e' => '\x1b',
        'f' => '\x0c',
        's' => ' ',
        'v' => '\x0b',
        'r' => '\r',
        'n' => '\n',
        't' => '\t',
        'u' => unicode(chars, 4, "invalid \\u argument")?,
        'U' => unicode(chars, 8, "invalid \\U argument")?,
        c => c,
    };
    word.push(c);
    Ok(())
}

/// The byte of an octal escape whose first digit is `first`: three digits,
/// the first at most 3, as tmux's.
fn octal(
    first: char,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Result<u8, TokenizeError> {
    let bad = TokenizeError::Escape("invalid octal escape");
    if first > '3' {
        return Err(bad);
    }
    let mut value = 0;
    for c in [Some(first), chars.next(), chars.next()] {
        value = value * 8 + c.and_then(|c| c.to_digit(8)).ok_or_else(|| bad.clone())?;
    }
    u8::try_from(value).map_err(|_| bad)
}

/// The character of a `\u` or `\U` escape: `size` hex digits.
fn unicode(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    size: usize,
    bad: &'static str,
) -> Result<char, TokenizeError> {
    let hex: String = chars.take(size).collect();
    if hex.len() != size || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(TokenizeError::Escape(bad));
    }
    u32::from_str_radix(&hex, 16)
        .ok()
        .and_then(char::from_u32)
        .ok_or(TokenizeError::Escape(bad))
}

fn expand_var(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    word: &mut String,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<(), TokenizeError> {
    let mut name = String::new();
    if chars.peek() == Some(&'{') {
        chars.next();
        loop {
            match chars.next() {
                Some('}') => break,
                Some(c) => name.push(c),
                None => return Err(TokenizeError::UnterminatedBrace),
            }
        }
    } else {
        // A name starts with a letter or `_`, as tmux's yylex_is_var: `$0`
        // and `$1x` stay as written.
        while let Some(&c) = chars.peek() {
            if c.is_ascii_alphabetic() || c == '_' || (!name.is_empty() && c.is_ascii_digit()) {
                name.push(c);
                chars.next();
            } else {
                break;
            }
        }
    }
    if name.is_empty() {
        word.push('$');
    } else {
        word.push_str(&env(&name).unwrap_or_default());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(name: &str) -> Option<String> {
        match name {
            "HOME" => Some("/home/u".into()),
            "X" => Some("ex".into()),
            _ => None,
        }
    }

    fn t(line: &str) -> Vec<Vec<String>> {
        tokenize(line, &env).unwrap()
    }

    #[test]
    fn spaces_and_tabs_part_words_and_a_line_ends_a_command() {
        assert_eq!(
            t("a\tb\u{a0}c\u{85}d\nx y\r\nz"),
            vec![w(&["a", "b\u{a0}c\u{85}d"]), w(&["x", "y"]), w(&["z"])]
        );
        assert_eq!(t("echo ~\nls"), vec![w(&["echo", "/home/u"]), w(&["ls"])]);
    }

    #[test]
    fn a_variable_name_starts_with_a_letter_or_underscore() {
        assert_eq!(
            t("echo $X $X1 \"$0\" $1x"),
            vec![w(&["echo", "ex", "", "$0", "$1x"])]
        );
    }

    #[test]
    fn join_args_survives_a_round_trip() {
        let words: Vec<String> = ["split-window", "-c", "#{pane_current_path}", "it's", ""]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let line = join_args(&words);
        let back = tokenize(&line, &|_| None).unwrap();
        assert_eq!(back, vec![words]);
    }

    fn w(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_block_is_one_word_of_commands() {
        // What tmux 3.6 stores for the same blocks (`show -v` of an option
        // set to one): the commands printed back, a new line's after ` ;; `.
        assert_eq!(
            t("if-shell -F 1 { display -p A ; display -p B }"),
            vec![w(&[
                "if-shell",
                "-F",
                "1",
                "display-message -p A ; display-message -p B"
            ])]
        );
        assert_eq!(
            t("bind -n F11 {\n  display -p one\n\n  display -p two\n}"),
            vec![w(&[
                "bind",
                "-n",
                "F11",
                "display-message -p one ;; display-message -p two"
            ])]
        );
        // Nested braces and formats inside survive, requoted as tmux's.
        assert_eq!(
            t(r#"set @z { display -p "q { x }" ; display '#{session_name}' }"#),
            vec![w(&[
                "set",
                "@z",
                r##"display-message -p "q { x }" ; display-message "#{session_name}""##
            ])]
        );
        // One that does not parse is kept as written.
        assert_eq!(
            t("set @y { nosuchcommand a\n nosuch b }"),
            vec![w(&["set", "@y", "nosuchcommand a ;; nosuch b"])]
        );
        // A block ends its word; a `;` after it ends the command.
        assert_eq!(t("a {b} ; c"), vec![w(&["a", "b"]), w(&["c"])]);
        assert_eq!(
            tokenize("a { b", &env),
            Err(TokenizeError::UnterminatedBlock)
        );
    }

    #[test]
    fn quoting_rules() {
        assert_eq!(
            t(r##"split-window -v -c "#{pane_current_path}""##),
            vec![w(&["split-window", "-v", "-c", "#{pane_current_path}"])]
        );
        assert_eq!(t("a 'b c' d"), vec![w(&["a", "b c", "d"])]);
        assert_eq!(t(r##"a "\"q\" \$X $X""##), vec![w(&["a", "\"q\" $X ex"])]);
        assert_eq!(t(r"a b\ c"), vec![w(&["a", "b c"])]);
        assert_eq!(
            t("cd ~/x ~ a~"),
            vec![w(&["cd", "/home/u/x", "/home/u", "a~"])]
        );
        assert_eq!(t("echo ${X}y $NOPE."), vec![w(&["echo", "exy", "."])]);
    }

    #[test]
    fn escape_arg_quotes_as_tmux_shows_options() {
        // What tmux 3.6's `show-options` printed for each value.
        let cases: &[(&str, &str)] = &[
            ("a b", r#""a b""#),
            ("x", "x"),
            ("a\"b", r#"'a"b'"#),
            ("it's", r#""it's""#),
            ("#{x}", r##""#{x}""##),
            ("", "''"),
            ("~/foo", r"\~/foo"),
            ("~", r"\~"),
            ("a~", "a~"),
            (" ", r#"" ""#),
            ("\"", r#"\""#),
            ("'", r"\'"),
            ("#", r"\#"),
            ("a;b", r#""a;b""#),
            ("a$b", r#""a\$b""#),
            ("tab\tx", r"tab\tx"),
            ("nl\nx", r"nl\nx"),
            ("a%b", r#""a%b""#),
            ("{", r"\{"),
            ("a{b", r#""a{b""#),
            (r"a\b", r"a\\b"),
            ("é ü", r#""é ü""#),
            ("a b\"c", r#""a b\"c""#),
            ("a`b", "a`b"),
            ("a\x01b", r"a\001b"),
            ("a\x1bb", r"a\033b"),
            ("a b\tc", r#""a b\tc""#),
            ("a\x7fb", r"a\177b"),
            ("~ x", r#""\~ x""#),
            (" ~", r#"" ~""#),
            ("a=b,c:d", "a=b,c:d"),
            ("[#{session_name}] ", r##""[#{session_name}] ""##),
        ];
        for (word, shown) in cases {
            assert_eq!(escape_arg(word), *shown, "{word:?}");
        }
        // And each reads back, but for a backslash in single quotes, which
        // tmux's does not read back either.
        for (word, shown) in cases {
            assert_eq!(t(&format!("x {shown}")), vec![w(&["x", word])], "{shown}");
        }
    }

    #[test]
    fn backslash_escapes_as_tmux_reads_them() {
        // What tmux 3.6 stored for each, set from a sourced file.
        let cases: &[(&str, &str)] = &[
            (r"a\tb", "a\tb"),
            (r#""a\tb""#, "a\tb"),
            (r"a\101b", "aAb"),
            (r#""a\101b""#, "aAb"),
            (r"a\eb", "a\x1bb"),
            (r"a\qb", "aqb"),
            (r#""a\qb""#, "aqb"),
            (r"a\\b", r"a\b"),
            (r#""a\$b""#, "a$b"),
            (r"aéb", "aéb"),
            (r"\~", "~"),
            (r"a\sb", "a b"),
            (r#""a\nb""#, "a\nb"),
            (r"'a\tb'", r"a\tb"),
            (r"a\012b", "a\nb"),
            (r"\303\251", "é"),
        ];
        for (line, word) in cases {
            assert_eq!(t(line), vec![w(&[word])], "{line}");
        }
        for bad in [r"a\0b", r"a\08b", r"a\4", r"\u00g1", r"a\"] {
            assert!(matches!(tokenize(bad, &env), Err(TokenizeError::Escape(_))));
        }
    }

    #[test]
    fn command_separators_and_comments() {
        assert_eq!(
            t("neww; splitw -h ;selectp -L"),
            vec![w(&["neww"]), w(&["splitw", "-h"]), w(&["selectp", "-L"])]
        );
        assert_eq!(t(r"bind x a \; b"), vec![w(&["bind", "x", "a", ";", "b"])]);
        assert_eq!(t("a # comment ; b"), vec![w(&["a"])]);
        assert_eq!(
            t("display #{session_name}"),
            vec![w(&["display", "#{session_name}"])]
        );
        assert_eq!(t("   "), Vec::<Vec<String>>::new());
    }

    #[test]
    fn unterminated_quotes_are_errors() {
        assert!(tokenize("a 'b", &env).is_err());
        assert!(tokenize("a \"b", &env).is_err());
        assert!(tokenize("a ${B", &env).is_err());
    }
}
