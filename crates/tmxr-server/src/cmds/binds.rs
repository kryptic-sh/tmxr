//! Key binding commands.

use std::fmt::Write as _;

use tmxr_command::{BindKey, Parsed};

use super::{Ctx, Outcome, join_args};
use crate::keys::BindSpec;
use crate::server::Server;

pub(super) fn run(
    srv: &mut Server,
    ctx: &Ctx,
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
            // A lone `;` word (`\;` in a command line) separates the bound
            // list's commands, as tmux's.
            let cmd = if pos.len() == 2 && pos[1].contains(' ') {
                pos[1].clone()
            } else {
                tmxr_command::split_arguments(&pos[1..])
                    .iter()
                    .map(|c| join_args(c))
                    .collect::<Vec<_>>()
                    .join(" ; ")
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
                // -q: a missing or unknown key is no error, as tmux's.
                let key = pos
                    .first()
                    .ok_or_else(|| "unbind-key needs a key".to_owned())
                    .and_then(|k| {
                        k.parse::<BindKey>()
                            .map_err(|e: tmxr_command::keys::UnknownKey| e.to_string())
                    });
                match key {
                    Ok(key) => {
                        srv.keys.unbind(table, &key);
                    }
                    Err(_) if a.has('q') => {}
                    Err(e) => return Err(e),
                }
            }
        }
        "list-keys" => list_keys(srv, ctx, a, pos, out)?,

        _ => return Ok(false),
    }
    Ok(true)
}

/// `list-keys`, as tmux 3.6's cmd_list_keys_exec: tables by name and each
/// in tmux's key order, as `bind-key` lines with the table and key padded to
/// the widest; or with -N the noted binds (with -a every one, its command
/// for a note) and no mouse keys, without -T the root table then the prefix
/// table, each key after the prefix key (or -P's text, or as many spaces)
/// and padded to the widest noted key. -1 takes each table's first, shown
/// on a client when there is one; a key no table binds is an error.
fn list_keys(
    srv: &mut Server,
    ctx: &Ctx,
    a: &tmxr_command::Args,
    pos: &[String],
    out: &mut Outcome,
) -> Result<(), String> {
    let only = pos
        .first()
        .map(|k| {
            k.parse::<BindKey>()
                .map_err(|_| format!("invalid key: {k}"))
        })
        .transpose()?;
    let all = srv.keys.list(None);
    let table = a.value('T');
    if let Some(t) = table
        && !all.iter().any(|(name, _, _)| name == t)
    {
        return Err(format!("table {t} doesn't exist"));
    }
    let width = |s: &str| ratatui::text::Line::raw(s).width();
    // A bind's commands as tmux prints them back.
    let shown = |cmd: &str| tmxr_command::print_command_list(cmd).unwrap_or_else(|| cmd.to_owned());
    let pad = |s: &str, w: usize| format!("{s}{}", " ".repeat(w.saturating_sub(width(s))));
    let listed: Vec<(String, BindKey, BindSpec)> = all
        .into_iter()
        .filter(|(name, key, _)| table.is_none_or(|t| t == name) && only.is_none_or(|k| k == *key))
        .collect();
    let mut lines = Vec::new();
    if a.has('N') {
        let noted = |b: &BindSpec| b.note.as_deref().is_some_and(|n| !n.is_empty());
        let in_table = |t: &str| -> Vec<&(String, BindKey, BindSpec)> {
            listed
                .iter()
                .filter(|(name, key, _)| name == t && !matches!(key, BindKey::Mouse(_)))
                .collect()
        };
        let key_width = |t: &str| {
            in_table(t)
                .into_iter()
                .filter(|(_, _, b)| noted(b))
                .map(|(_, k, _)| width(&k.to_string()))
                .max()
                .unwrap_or(0)
        };
        let mut notes = |t: &str, keywidth: usize, start: &str| {
            for (_, key, b) in in_table(t)
                .into_iter()
                .filter(|(_, _, b)| noted(b) || a.has('a'))
            {
                let note = if noted(b) {
                    b.note.clone().unwrap_or_default()
                } else {
                    shown(&b.cmd)
                };
                lines.push(format!(
                    "{start}{}{note}",
                    pad(&key.to_string(), keywidth + 1)
                ));
                if a.has('1') {
                    break;
                }
            }
        };
        if let Some(t) = table {
            let start = a.value('P').unwrap_or_default();
            notes(t, key_width(t), start);
        } else {
            let prefix = srv.cfg.session.prefix.to_string();
            let mut with_prefix = prefix != "None";
            let start = match a.value('P') {
                Some(p) => p.to_owned(),
                None if with_prefix => format!("{prefix} "),
                None => String::new(),
            };
            let mut keywidth = key_width("root");
            if with_prefix {
                match key_width("prefix") {
                    0 => with_prefix = false,
                    w => keywidth = keywidth.max(w),
                }
            }
            notes("root", keywidth, &" ".repeat(width(&start)));
            if with_prefix {
                notes("prefix", keywidth, &start);
            }
        }
    } else {
        let repeat = listed.iter().any(|(_, _, b)| b.repeat);
        let tablewidth = listed.iter().map(|(t, _, _)| width(t)).max().unwrap_or(0);
        let escaped = |k: &BindKey| tmxr_command::escape_arg(&k.to_string());
        let keywidth = listed
            .iter()
            .map(|(_, k, _)| width(&escaped(k)))
            .max()
            .unwrap_or(0);
        let mut last_table = None;
        for (t, key, b) in &listed {
            // -1: each table's first.
            if a.has('1') && last_table == Some(t) {
                continue;
            }
            last_table = Some(t);
            let r = match (repeat, b.repeat) {
                (false, _) => "",
                (true, true) => "-r ",
                (true, false) => "   ",
            };
            lines.push(format!(
                "bind-key {r}-T {} {} {}",
                pad(t, tablewidth),
                pad(&escaped(key), keywidth),
                shown(&b.cmd)
            ));
        }
    }
    if let Some(k) = only
        && lines.is_empty()
    {
        return Err(format!("unknown key: {k}"));
    }
    match super::display_client(srv, ctx).filter(|_| a.has('1')) {
        Some(c) => {
            for line in lines {
                srv.show_message(c, line);
            }
        }
        None => {
            for line in lines {
                let _ = writeln!(out.stdout, "{line}");
            }
        }
    }
    Ok(())
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
