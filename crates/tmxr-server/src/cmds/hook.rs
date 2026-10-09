//! `set-hook` and `show-hooks`.

use std::fmt::Write as _;

use tmxr_command::Parsed;

use super::{Ctx, Outcome};
use crate::hooks::HookTable;
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
        "set-hook" => {
            if a.has('p') || a.has('w') {
                return Err("set-hook: pane and window hooks are not supported".into());
            }
            let name = &pos[0];
            if !crate::hooks::known(name) {
                return Err(format!("unknown hook: {name}"));
            }
            if a.has('R') {
                srv.queue_hook(name, ctx.clone(), None);
                return Ok(true);
            }
            let table = hooks_mut(srv, ctx, a.has('g'), a.value('t'))?;
            if a.has('u') {
                table.remove(name);
            } else {
                let cmd = pos
                    .get(1)
                    .ok_or_else(|| format!("set-hook {name}: no command"))?;
                let list = table.entry(name.clone()).or_default();
                if !a.has('a') {
                    list.clear();
                }
                list.push(cmd.clone());
            }
        }
        "show-hooks" => {
            let table = if a.has('g') {
                &srv.hooks
            } else {
                let sid = target::session(srv, ctx, a.value('t'))?;
                &srv.sessions.get(&sid).ok_or("no such session")?.hooks
            };
            for (name, cmds) in table {
                for (i, cmd) in cmds.iter().enumerate() {
                    let _ = writeln!(out.stdout, "{name}[{i}] {cmd}");
                }
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// The global table with `-g`, else the target (or current) session's.
fn hooks_mut<'s>(
    srv: &'s mut Server,
    ctx: &Ctx,
    global: bool,
    target: Option<&str>,
) -> Result<&'s mut HookTable, String> {
    if global {
        return Ok(&mut srv.hooks);
    }
    let sid = target::session(srv, ctx, target)?;
    Ok(&mut srv.sessions.get_mut(&sid).ok_or("no such session")?.hooks)
}
