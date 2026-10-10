//! Key binding commands.

use std::fmt::Write as _;

use tmxr_command::{BindKey, Parsed};

use super::{Ctx, Outcome, join_args};
use crate::keys::BindSpec;
use crate::server::Server;

pub(super) fn run(
    srv: &mut Server,
    _ctx: &Ctx,
    p: &Parsed,
    out: &mut Outcome,
) -> Result<bool, String> {
    let a = &p.args;
    let pos = a.positional();
    match p.name() {
        "bind-key" => {
            let table = if a.has('n') {
                "root"
            } else {
                a.value('T').unwrap_or("prefix")
            };
            let key: BindKey = pos[0]
                .parse()
                .map_err(|e: tmxr_command::keys::UnknownKey| e.to_string())?;
            let cmd = if pos.len() == 2 && pos[1].contains(' ') {
                pos[1].clone()
            } else {
                join_args(&pos[1..])
            };
            if cmd.is_empty() {
                // `bind -N note key` with no command only changes the note.
                if let Some(b) = srv.keys.get(table, &key).cloned() {
                    srv.keys.bind(
                        table,
                        key,
                        BindSpec {
                            note: a.value('N').map(str::to_owned),
                            ..b
                        },
                    );
                }
                return Ok(true);
            }
            srv.keys.bind(
                table,
                key,
                BindSpec {
                    cmd,
                    note: a.value('N').map(str::to_owned),
                    repeat: a.has('r'),
                },
            );
        }
        "unbind-key" => {
            let table = if a.has('n') {
                "root"
            } else {
                a.value('T').unwrap_or("prefix")
            };
            if a.has('a') {
                srv.keys.unbind_all(table);
            } else {
                let key: BindKey = pos
                    .first()
                    .ok_or("unbind-key needs a key")?
                    .parse()
                    .map_err(|e: tmxr_command::keys::UnknownKey| e.to_string())?;
                srv.keys.unbind(table, &key);
            }
        }
        "list-keys" => {
            let only_notes = a.has('N');
            let table = a.value('T');
            let key = pos
                .first()
                .map(|k| k.parse::<BindKey>().map_err(|e| e.to_string()))
                .transpose()?;
            let mut binds = srv.keys.list(table);
            if let Some(k) = key {
                binds.retain(|(_, bk, _)| *bk == k);
            }
            if a.has('1') {
                // The bind a key press reaches first: prefix, then root, then
                // the rest (copy mode).
                let rank = |t: &str| match t {
                    "prefix" => 0,
                    "root" => 1,
                    _ => 2,
                };
                binds.sort_by_key(|(t, _, _)| rank(t));
                binds.truncate(1);
            }
            if binds.is_empty()
                && let Some(k) = key
            {
                let _ = writeln!(out.stdout, "{k} is not bound");
            }
            let width = binds
                .iter()
                .map(|(_, k, _)| k.to_string().len())
                .max()
                .unwrap_or(0);
            for (table, key, b) in binds {
                if only_notes {
                    let Some(note) = &b.note else { continue };
                    let prefix = match table.as_str() {
                        "prefix" => format!("{} ", srv.prefix),
                        "root" => String::new(),
                        t => format!("[{t}] "),
                    };
                    let k = format!("{prefix}{key}");
                    let _ = writeln!(out.stdout, "{k:<w$}  {note}", w = width + 6);
                } else {
                    let _ = writeln!(out.stdout, "{}", bind_line(&table, &key, &b));
                }
            }
        }

        _ => return Ok(false),
    }
    Ok(true)
}

/// A bind as `list-keys` prints it: the `bind-key` command that makes it.
pub fn bind_line(table: &str, key: &BindKey, b: &BindSpec) -> String {
    let r = if b.repeat { "-r " } else { "" };
    format!(
        "bind-key {r}-T {table} {} {}",
        join_args(&[key.to_string()]),
        b.cmd
    )
}
