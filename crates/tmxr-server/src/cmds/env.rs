//! The environment commands: `set-environment` and `show-environment`.

use std::fmt::Write as _;

use tmxr_command::Parsed;

use super::{Ctx, Outcome, expand_for};
use crate::model::Environment;
use crate::server::Server;
use crate::target;

pub(super) fn run(
    srv: &mut Server,
    ctx: &Ctx,
    p: &Parsed,
    out: &mut Outcome,
) -> Result<bool, String> {
    let a = &p.args;
    let pos = a.positional();
    match p.name() {
        "set-environment" => {
            let name = &pos[0];
            if name.is_empty() || name.contains('=') {
                return Err(format!("bad variable name: {name}"));
            }
            let value = pos.get(1).map(|v| {
                if a.has('F') {
                    expand_for(srv, ctx, ctx.pane, v)
                } else {
                    v.clone()
                }
            });
            let env = if a.has('g') {
                &mut srv.global_env
            } else {
                let sid = target::session(srv, ctx, a.value('t'))?;
                &mut srv.sessions.get_mut(&sid).ok_or("no such session")?.env
            };
            if a.has('u') {
                env.unset(name);
            } else if a.has('r') {
                env.set(name, None);
            } else {
                let value = value.ok_or_else(|| format!("no value for {name}"))?;
                env.set(name, Some(value));
            }
        }
        "show-environment" => {
            let env = if a.has('g') {
                // The server's own environment, with the global changes over
                // it: what a new pane starts from.
                let process: Environment = std::env::vars().collect();
                srv.global_env.over(&process)
            } else {
                let sid = target::session(srv, ctx, a.value('t'))?;
                srv.sessions.get(&sid).ok_or("no such session")?.env.clone()
            };
            let name = pos.first();
            let mut shown = 0;
            for (k, v) in env.iter().filter(|(k, _)| name.is_none_or(|n| n == k)) {
                shown += 1;
                let line = match (a.has('s'), v) {
                    (false, Some(v)) => format!("{k}={v}"),
                    (false, None) => format!("-{k}"),
                    (true, Some(v)) => format!("{k}=\"{}\"; export {k};", shell_escape(v)),
                    (true, None) => format!("unset {k};"),
                };
                let _ = writeln!(out.stdout, "{line}");
            }
            if let (Some(n), 0) = (name, shown) {
                return Err(format!("unknown variable: {n}"));
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// A value inside double quotes for a POSIX shell, as tmux's
/// `show-environment -s` writes it: `"`, `$`, `` ` `` and `\` escaped.
fn shell_escape(v: &str) -> String {
    let mut s = String::with_capacity(v.len());
    for c in v.chars() {
        if matches!(c, '"' | '$' | '`' | '\\') {
            s.push('\\');
        }
        s.push(c);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_values_escape_what_double_quotes_do_not() {
        assert_eq!(
            shell_escape(r#"a "b" $c `d` \e"#),
            r#"a \"b\" \$c \`d\` \\e"#
        );
        assert_eq!(shell_escape("plain"), "plain");
    }
}
