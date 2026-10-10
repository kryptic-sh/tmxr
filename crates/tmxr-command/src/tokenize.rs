//! Split a tmux command line into a command list.
//!
//! Follows tmux's quoting: words split on whitespace; `'…'` is literal;
//! `"…"` allows `\"`, `\\`, `\$` escapes and `$VAR` / `${VAR}` expansion;
//! outside quotes `\x` is a literal `x`, `~` at the start of a word is
//! `$HOME`, and `$VAR` expands. A `;` ending a word (or standing alone)
//! separates commands; `\;` is a literal `;`. A word starting with `#` (other
//! than a `#{format}`) starts a comment. `{ … }` is one word holding the
//! commands inside, joined by ` ; `, as tmux passes a block of commands to
//! `if-shell`, `bind-key` and the like.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenizeError {
    #[error("unterminated {0} quote")]
    Unterminated(char),
    #[error("unterminated ${{ in variable reference")]
    UnterminatedBrace,
    #[error("unterminated {{ … }} block")]
    UnterminatedBlock,
}

/// Parse `line` into commands, each a list of words. `env` resolves
/// `$VAR` references (unset variables expand to nothing, as in a shell).
pub fn tokenize(
    line: &str,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<Vec<String>>, TokenizeError> {
    let mut commands = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = line.chars().peekable();

    let end_command = |words: &mut Vec<String>, commands: &mut Vec<Vec<String>>| {
        if !words.is_empty() {
            commands.push(std::mem::take(words));
        }
    };

    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '#' if !in_word && chars.peek() != Some(&'{') => break,
            '{' if !in_word => words.push(block(&mut chars)?),
            ';' => {
                // `;` ends the current word and the command.
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
                end_command(&mut words, &mut commands);
            }
            '\\' => {
                in_word = true;
                if let Some(n) = chars.next() {
                    word.push(n);
                }
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
                        Some('\\') => match chars.next() {
                            Some(n @ ('"' | '\\' | '$')) => word.push(n),
                            Some(n) => {
                                word.push('\\');
                                word.push(n);
                            }
                            None => return Err(TokenizeError::Unterminated('"')),
                        },
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
                    .is_none_or(|n| *n == '/' || n.is_whitespace() || *n == ';');
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
    if in_word {
        words.push(word);
    }
    end_command(&mut words, &mut commands);
    Ok(commands)
}

/// The commands in a `{ … }` block, its `{` already read, up to the
/// matching `}`: one per line or `;`, joined by ` ; ` to be parsed again
/// when they run. Quotes, escapes and nested braces (`#{format}` among them)
/// are kept as written.
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
    Ok(lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ; "))
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
        while let Some(&c) = chars.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
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

    fn w(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_block_is_one_word_of_commands() {
        // What tmux 3.6 stores for the same blocks.
        assert_eq!(
            t("if-shell -F 1 { display -p A ; display -p B }"),
            vec![w(&["if-shell", "-F", "1", "display -p A ; display -p B"])]
        );
        assert_eq!(
            t("bind -n F11 {\n  display -p one\n\n  display -p two\n}"),
            vec![w(&["bind", "-n", "F11", "display -p one ; display -p two"])]
        );
        // Quotes, nested braces and formats inside are kept as written.
        assert_eq!(
            t(r#"set @z { display -p "q { x }" ; display '#{session_name}' }"#),
            vec![w(&[
                "set",
                "@z",
                r#"display -p "q { x }" ; display '#{session_name}'"#
            ])]
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
