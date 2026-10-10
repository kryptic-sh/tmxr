//! `wait-for`: a command list waits on a channel for a signal or its lock,
//! from a command client, a bind or anywhere else commands run.

use std::time::Duration;

use tmxr_command::Parsed;

use super::{Ctx, Outcome};
use crate::jobs::{Job, Work};
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
    // The rest of the list waits as a job, queued on the channel.
    let wait = |w| Job {
        delay: Duration::ZERO,
        work: Work::Channel(w),
        rest: Vec::new(),
    };
    if a.has('S') {
        for done in srv.waits.signal(&name) {
            srv.job_done(done);
        }
    } else if a.has('U') {
        if let Some(done) = srv.waits.unlock(&name)? {
            srv.job_done(done);
        }
    } else if a.has('L') {
        if !srv.waits.lock(&name) {
            out.job = Some(wait(Wait::Lock(name)));
        }
    } else if !srv.waits.wait(&name) {
        out.job = Some(wait(Wait::Signal(name)));
    }
    Ok(true)
}
