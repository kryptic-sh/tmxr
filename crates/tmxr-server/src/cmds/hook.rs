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
            let name = &pos[0];
            if !crate::hooks::known(name) {
                return Err(format!("unknown hook: {name}"));
            }
            if a.has('R') {
                srv.queue_hook(name, ctx.clone(), crate::hooks::HookScope::default());
                return Ok(true);
            }
            let table = hooks_mut(srv, ctx, a)?;
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
            let table = hooks_mut(srv, ctx, a)?;
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

/// The table a command means: the global one with `-g`, the target pane's
/// with `-p`, its window's with `-w`, else the target session's.
fn hooks_mut<'s>(
    srv: &'s mut Server,
    ctx: &Ctx,
    a: &tmxr_command::Args,
) -> Result<&'s mut HookTable, String> {
    let t = a.value('t');
    if a.has('g') {
        return Ok(&mut srv.hooks);
    }
    if a.has('p') {
        let (_, _, pid) = target::pane(srv, ctx, t)?;
        return Ok(&mut srv.panes.get_mut(&pid).ok_or("no such pane")?.hooks);
    }
    if a.has('w') {
        let (_, _, wid) = target::window(srv, ctx, t)?;
        return Ok(&mut srv.windows.get_mut(&wid).ok_or("no such window")?.hooks);
    }
    let sid = target::session(srv, ctx, t)?;
    Ok(&mut srv.sessions.get_mut(&sid).ok_or("no such session")?.hooks)
}
