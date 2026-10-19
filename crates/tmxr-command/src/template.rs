//! tmux's command templates: answers put into `%%` and `%1` … `%9`, as
//! `command-prompt`, `display-panes` and the choosers fill theirs.

/// The characters a quoted placeholder (`%1%`, `%%%`) puts a backslash
/// before.
const QUOTED: &[char] = &['"', '\\', '$', ';', '~'];

/// `template` with answer `s`, number `idx` (from 1), put in: every `%idx`,
/// and the first `%%` an earlier answer has not taken; a `%` straight after
/// either quotes the answer for the command parser. A transcription of
/// tmux's cmd_template_replace (cmd.c).
pub fn template_replace(template: &str, s: &str, idx: usize) -> String {
    let mut out = String::with_capacity(template.len() + s.len());
    let mut chars = template.chars().peekable();
    let mut replaced = false;
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let numbered = chars
            .peek()
            .and_then(|n| n.to_digit(10))
            .is_some_and(|n| (1..=9).contains(&n) && n as usize == idx);
        let first_pair = !numbered && chars.peek() == Some(&'%') && !replaced;
        if !numbered && !first_pair {
            out.push(c);
            continue;
        }
        replaced |= first_pair;
        chars.next();
        let quoted = chars.next_if_eq(&'%').is_some();
        for a in s.chars() {
            if quoted && QUOTED.contains(&a) {
                out.push('\\');
            }
            out.push(a);
        }
    }
    out
}

/// `template` with each answer put in by its number, as tmux's
/// args_make_commands does before parsing it.
pub fn fill_template(template: &str, answers: &[String]) -> String {
    answers
        .iter()
        .enumerate()
        .fold(template.to_owned(), |t, (i, a)| {
            template_replace(&t, a, i + 1)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(template: &str, answers: &[&str]) -> String {
        let answers: Vec<String> = answers.iter().map(|a| (*a).to_owned()).collect();
        fill_template(template, &answers)
    }

    #[test]
    fn answers_fill_numbers_and_pairs_as_tmuxs() {
        // Each as tmux 3.6's command-prompt filled it (before parsing).
        assert_eq!(
            fill("set -g @r '%1|%2|%%|%%'", &["x", "y"]),
            "set -g @r 'x|y|x|y'"
        );
        assert_eq!(fill("%%-%%-%1", &["x", "y"]), "x-y-x");
        assert_eq!(fill("'%1%'", &[r#"p\$q;r"s"#]), r#"'p\\\$q\;r\"s'"#);
        assert_eq!(fill("\"%%%\"", &[r"p\$q"]), r#""p\\\$q""#);
        assert_eq!(fill("%1|%2", &["x,y"]), "x,y|%2");
        assert_eq!(fill("%1|%2", &["x", ""]), "x|");
        assert_eq!(fill("50% off %3", &["x"]), "50% off %3");
    }
}
