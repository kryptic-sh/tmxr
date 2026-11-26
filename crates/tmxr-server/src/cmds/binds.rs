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
            // list's commands, as tmux's; a `{ … }` block argument is kept
            // in braces, so it runs and prints as one.
            let cmd = if pos.len() == 2 && pos[1].contains(' ') {
                pos[1].clone()
            } else {
                let bound = tmxr_command::Line {
                    words: pos[1..].to_vec(),
                    blocks: (1..pos.len()).map(|j| a.is_block(j)).collect(),
                    new_group: false,
                };
                tmxr_command::split_line(&bound)
                    .iter()
                    .map(|c| {
                        c.words
                            .iter()
                            .zip(&c.blocks)
                            .map(|(w, &block)| {
                                if block {
                                    format!("{{ {w} }}")
                                } else {
                                    join_args(std::slice::from_ref(w))
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
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

/// `list-keys`, as tmux 3.8's cmd_list_keys_exec: tables by name and each
/// in tmux's key order, as `bind-key` lines with the table and key padded to
/// the widest; or with -N the noted binds (with -a every one, its command
/// for a note), without -T the prefix table then the root
/// table, each key after the prefix key (or -P's text) and a space, padded
/// to the widest. -1 takes the first, shown on a client when there is one;
/// a key no table binds is an error.
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
    // A bind the client's terminal cannot send is listed with a comment
    // saying what it needs.
    let tier = marked_for(srv, ctx);
    let mark = |key: &BindKey| {
        tier.and_then(|t| crate::keyboard::needs_comment(crate::keyboard::needs(key), t))
            .map_or_else(String::new, |c| format!(" {c}"))
    };
    let notes = a.has('N');
    let noted = |b: &BindSpec| b.note.as_deref().is_some_and(|n| !n.is_empty());
    let mut listed: Vec<(String, BindKey, BindSpec)> = all
        .into_iter()
        .filter(|(name, key, b)| {
            only.is_none_or(|k| k == *key)
                && match (notes, table) {
                    (_, Some(t)) => t == name,
                    (false, None) => true,
                    (true, None) => name == "prefix" || name == "root",
                }
                && (!notes || noted(b) || a.has('a'))
        })
        .collect();
    // -N without -T: the prefix table first, as tmux's
    // cmd_list_keys_get_root_and_prefix.
    if notes && table.is_none() {
        listed.sort_by_key(|(name, _, _)| name != "prefix");
    }
    // -1: the first alone, which is all the widths are taken over.
    if a.has('1') {
        listed.truncate(1);
    }
    let escaped = |k: &BindKey| tmxr_command::escape_arg(&k.to_string());
    let mut lines = Vec::new();
    if notes {
        let start = match a.value('P') {
            Some(p) => p.to_owned(),
            None => Some(srv.cfg.session.prefix.to_string())
                .filter(|p| p != "None")
                .unwrap_or_default(),
        };
        let keywidth = listed
            .iter()
            .map(|(_, k, _)| width(&k.to_string()))
            .max()
            .unwrap_or(0);
        for (_, key, b) in &listed {
            let note = if noted(b) {
                b.note.clone().unwrap_or_default()
            } else {
                shown(&b.cmd)
            };
            lines.push(format!(
                "{start} {} {note}{}",
                pad(&key.to_string(), keywidth),
                mark(key)
            ));
        }
    } else {
        let repeat = listed.iter().any(|(_, _, b)| b.repeat);
        let tablewidth = listed.iter().map(|(t, _, _)| width(t)).max().unwrap_or(0);
        let keywidth = listed
            .iter()
            .map(|(_, k, _)| width(&escaped(k)))
            .max()
            .unwrap_or(0);
        for (t, key, b) in &listed {
            // tmux's template prints `#{?key_repeat,-r,  }` only when some
            // bind repeats, and a space either way.
            let r = match (repeat, b.repeat) {
                (false, _) => " ",
                (true, true) => "-r ",
                (true, false) => "   ",
            };
            lines.push(format!(
                "bind-key {r}-T {} {} {}{}",
                pad(t, tablewidth),
                pad(&escaped(key), keywidth),
                shown(&b.cmd),
                mark(key)
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

/// The keyboard tier of the client `list-keys` marks binds for: the
/// attached client that ran it itself, from a bind or the command prompt.
/// `None` for a command client, run in a pane (a shell, a script, a
/// `run-shell` plugin) or outside tmxr, which prints no marks, as tmux
/// prints none.
fn marked_for(srv: &Server, ctx: &Ctx) -> Option<crate::keyboard::Keyboard> {
    let id = super::attached_client(srv, ctx)?;
    srv.clients.get(&id)?.att.as_ref().map(|a| a.keyboard)
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
