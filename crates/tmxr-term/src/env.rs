//! This process's environment as text.
//!
//! [`std::env::vars`] panics on a variable whose name or value is not valid
//! Unicode, which a parent may set; the client's hello and
//! `show-environment -g` read the environment, so one such variable crashed
//! the client or the whole server. [`vars`] leaves those variables out
//! instead: tmxr passes environments around as text and has nowhere to keep
//! the raw bytes. Panes inherit the server's environment unchanged, these
//! variables included (`pty`).

use std::ffi::OsString;

/// This process's environment variables whose name and value are valid
/// Unicode; the others are left out.
#[must_use]
pub fn vars() -> Vec<(String, String)> {
    text_only(std::env::vars_os())
}

fn text_only(vars: impl Iterator<Item = (OsString, OsString)>) -> Vec<(String, String)> {
    vars.filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name or value that is not valid Unicode on this platform.
    fn not_text() -> OsString {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt as _;
            OsString::from_vec(vec![b'a', 0xff])
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt as _;
            // An unpaired surrogate.
            OsString::from_wide(&[u16::from(b'a'), 0xd800])
        }
    }

    #[test]
    fn variables_that_are_not_text_are_left_out() {
        let vars = vec![
            (OsString::from("KEEP"), OsString::from("yes")),
            (OsString::from("BAD_VALUE"), not_text()),
            (not_text(), OsString::from("bad name")),
        ];
        assert_eq!(
            text_only(vars.into_iter()),
            [("KEEP".to_owned(), "yes".to_owned())]
        );
    }
}
