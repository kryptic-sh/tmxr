//! `wait-for`, between command clients.

use tmxr_command::Parsed;

use super::{Ctx, Outcome};
use crate::server::Server;
use crate::waits::Wait;

pub(super) fn run(
    srv: &mut Server,
    _ctx: &Ctx,
    p: &Parsed,
    out: &mut Outcome,
) -> Result<bool, String> {
    if p.name() != "wait-for" {
        return Ok(false);
    }
    let a = &p.args;
    let name = a.positional()[0].clone();
    if a.has('S') {
        for client in srv.waits.signal(&name) {
            srv.release_waiter(client);
        }
    } else if a.has('U') {
        if let Some(client) = srv.waits.unlock(&name)? {
            srv.release_waiter(client);
        }
    } else if a.has('L') {
        if !srv.waits.lock(&name) {
            out.wait = Some(Wait::Lock(name));
        }
    } else if !srv.waits.wait(&name) {
        out.wait = Some(Wait::Signal(name));
    }
    Ok(true)
}
