//! getopt-style flag parsing with tmux's flag specs.
//!
//! A spec lists the accepted flag letters; a letter followed by `:` takes a
//! value (`"bc:dhl:t:v"`). Flags may be bundled (`-dh`), values may be
//! attached (`-c/tmp`) or separate (`-c /tmp`), and `--` ends the flags.
//! Parsing stops at the first positional argument, as in tmux, so a command
//! given to `new-window` keeps its own flags.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArgsError {
    #[error("unknown flag -{0}")]
    UnknownFlag(char),
    #[error("-{0} expects an argument")]
    MissingValue(char),
}

/// Parsed flags and positional arguments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    flags: BTreeMap<char, Option<String>>,
    positional: Vec<String>,
}

impl Args {
    pub fn parse(spec: &str, argv: &[String]) -> Result<Self, ArgsError> {
        let mut out = Self::default();
        let mut i = 0;
        while i < argv.len() {
            let arg = &argv[i];
            if arg == "--" {
                i += 1;
                break;
            }
            let Some(letters) = arg.strip_prefix('-').filter(|l| !l.is_empty()) else {
                break;
            };
            for (pos, c) in letters.char_indices() {
                let takes_value = match spec.find(c) {
                    Some(at) => spec[at + c.len_utf8()..].starts_with(':'),
                    None => return Err(ArgsError::UnknownFlag(c)),
                };
                if c == ':' {
                    return Err(ArgsError::UnknownFlag(c));
                }
                if !takes_value {
                    out.flags.insert(c, None);
                    continue;
                }
                let attached = &letters[pos + c.len_utf8()..];
                let value = if attached.is_empty() {
                    i += 1;
                    argv.get(i).cloned().ok_or(ArgsError::MissingValue(c))?
                } else {
                    attached.to_owned()
                };
                out.flags.insert(c, Some(value));
                break;
            }
            i += 1;
        }
        out.positional = argv[i..].to_vec();
        Ok(out)
    }

    pub fn has(&self, flag: char) -> bool {
        self.flags.contains_key(&flag)
    }

    pub fn value(&self, flag: char) -> Option<&str> {
        self.flags.get(&flag).and_then(|v| v.as_deref())
    }

    pub fn positional(&self) -> &[String] {
        &self.positional
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn bundled_attached_and_separate_values() {
        let a = Args::parse("bc:dhl:t:v", &v(&["-dh", "-c/tmp", "-t", "x", "vim", "-c"])).unwrap();
        assert!(a.has('d') && a.has('h') && !a.has('v'));
        assert_eq!(a.value('c'), Some("/tmp"));
        assert_eq!(a.value('t'), Some("x"));
        assert_eq!(a.positional(), &["vim".to_owned(), "-c".to_owned()]);
    }

    #[test]
    fn value_flag_ends_a_bundle() {
        let a = Args::parse("c:h", &v(&["-hc", "dir"])).unwrap();
        assert!(a.has('h'));
        assert_eq!(a.value('c'), Some("dir"));
    }

    #[test]
    fn double_dash_and_errors() {
        let a = Args::parse("h", &v(&["--", "-h"])).unwrap();
        assert!(!a.has('h'));
        assert_eq!(a.positional(), &["-h".to_owned()]);
        assert_eq!(
            Args::parse("h", &v(&["-x"])).unwrap_err(),
            ArgsError::UnknownFlag('x')
        );
        assert_eq!(
            Args::parse("c:", &v(&["-c"])).unwrap_err(),
            ArgsError::MissingValue('c')
        );
        // A lone "-" is positional, as in tmux.
        assert_eq!(
            Args::parse("h", &v(&["-"])).unwrap().positional(),
            &["-".to_owned()]
        );
    }
}
