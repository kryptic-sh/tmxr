//! Save and restore sessions, à la tmux-resurrect.
//!
//! A save is a versioned JSON file in `<data dir>/tmxr/resurrect/<server>/`
//! (one directory per socket, see [`dir`]), plus a `last` file naming the
//! newest one. Restored panes start in their saved directory; a pane whose
//! foreground program is in `resurrect.processes` starts that program again
//! (by name — arguments are not saved, so a restore never re-runs a saved
//! command line). See `docs/plan/11-resurrect.md`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use hjkl_layout::{LayoutTree, SplitDir};
use serde::{Deserialize, Serialize};
use tmxr_proto::socket::Endpoint;

use crate::model::{PaneId, SessionId, WindowId};
use crate::server::Server;

const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Save {
    pub version: u32,
    pub sessions: Vec<SavedSession>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedSession {
    pub name: String,
    pub cwd: PathBuf,
    pub current: u32,
    pub last: Option<u32>,
    pub windows: Vec<SavedWindow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedWindow {
    pub index: u32,
    pub name: String,
    pub auto_name: bool,
    pub zoomed: bool,
    pub synchronize: bool,
    /// Position of the active pane in `panes`.
    pub active: usize,
    /// Position of the window's last pane (`select-pane -l`), if any. Saves
    /// from before this field restore without one.
    #[serde(default)]
    pub last: Option<usize>,
    pub layout: SavedLayout,
    pub panes: Vec<SavedPane>,
    /// A window linked into this session that an earlier session in the save
    /// holds too: restored by linking that one, not as a copy (its own
    /// layout and panes are left empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<SavedLink>,
    /// `resize-window`'s size (columns, rows), when it set one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_size: Option<(u16, u16)>,
}

/// Where a linked window was saved first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedLink {
    pub session: String,
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedPane {
    pub cwd: PathBuf,
    /// Program to start again, when it is in `resurrect.processes`.
    pub command: Option<String>,
    /// Its arguments, when it is in `resurrect.restore-args`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Its title (`#{pane_title}`), restored until its program sets one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Layout shape; leaves are positions in [`SavedWindow::panes`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SavedLayout {
    Leaf(usize),
    Split {
        /// `true` for side by side (tmux `-h`).
        horizontal: bool,
        ratio: f32,
        a: Box<Self>,
        b: Box<Self>,
    },
}

fn save_layout(tree: &LayoutTree, panes: &[PaneId]) -> SavedLayout {
    match tree {
        LayoutTree::Split {
            dir, ratio, a, b, ..
        } => SavedLayout::Split {
            horizontal: *dir == SplitDir::Vertical,
            ratio: *ratio,
            a: Box::new(save_layout(a, panes)),
            b: Box::new(save_layout(b, panes)),
        },
        LayoutTree::Leaf(id) => {
            SavedLayout::Leaf(panes.iter().position(|p| *p as usize == *id).unwrap_or(0))
        }
        _ => SavedLayout::Leaf(0),
    }
}

fn load_layout(l: &SavedLayout, panes: &[PaneId]) -> Option<LayoutTree> {
    Some(match l {
        SavedLayout::Leaf(i) => LayoutTree::Leaf(*panes.get(*i)? as usize),
        SavedLayout::Split {
            horizontal,
            ratio,
            a,
            b,
        } => LayoutTree::split(
            if *horizontal {
                SplitDir::Vertical
            } else {
                SplitDir::Horizontal
            },
            ratio.clamp(0.05, 0.95),
            load_layout(a, panes)?,
            load_layout(b, panes)?,
        ),
    })
}

/// Saves of the server at `endpoint`. Each server keeps its own, so a
/// throwaway `-L` server neither restores nor overwrites the main one's.
pub fn dir(endpoint: &Endpoint) -> Result<PathBuf, String> {
    hjkl_xdg::data_dir("tmxr")
        .map(|d| d.join("resurrect").join(endpoint.slug()))
        .map_err(|e| e.to_string())
}

/// The pane's foreground program, read now: the cache `refresh_commands`
/// keeps is only as fresh as the last status tick, which a save right after
/// a program started would miss.
pub fn foreground(srv: &Server, pane: PaneId) -> Option<String> {
    srv.panes
        .get(&pane)
        .and_then(|p| tmxr_term::process::foreground_command(&p.pty))
        .or_else(|| srv.commands.get(&pane).cloned())
}

/// The arguments of `command`, running in `pane`, when they are to be
/// restored: it is in `resurrect.restore-args` and is still the pane's
/// foreground program (the name was read earlier).
fn saved_args(srv: &Server, pane: PaneId, command: &str) -> Vec<String> {
    if !srv.cfg.resurrect.restore_args.iter().any(|c| c == command) {
        return Vec::new();
    }
    let Some(argv) = srv
        .panes
        .get(&pane)
        .and_then(|p| tmxr_term::process::foreground_args(&p.pty))
    else {
        return Vec::new();
    };
    match argv.split_first() {
        Some((program, args)) if program_name(program).eq_ignore_ascii_case(command) => {
            args.to_vec()
        }
        _ => Vec::new(),
    }
}

/// What to start again in `pane`, by `resurrect.processes`: a program and
/// its arguments. Each entry is one of tmux-resurrect's forms, and the first
/// that matches the pane's foreground program wins:
/// - `name`: that program, with its arguments when it is in `restore-args`;
/// - `~text`: a command line holding `text`, restored whole;
/// - `match->command`: a command line starting with `match` (holding it, with
///   a leading `~`), restored as `command`, split as tmux splits a command.
fn saved_program(srv: &Server, pane: PaneId) -> Option<(String, Vec<String>)> {
    let name = foreground(srv, pane)?;
    // The foreground program's argv, read once and only when an entry needs
    // the command line.
    let argv = std::cell::OnceCell::new();
    let argv = || {
        argv.get_or_init(|| {
            srv.panes
                .get(&pane)
                .and_then(|p| tmxr_term::process::foreground_args(&p.pty))
                .filter(|a| {
                    a.first()
                        .is_some_and(|p| program_name(p).eq_ignore_ascii_case(&name))
                })
        })
    };
    let line = || argv().as_ref().map(|a| a.join(" "));
    for entry in &srv.cfg.resurrect.processes {
        let (pattern, restore) = match entry.split_once("->") {
            Some((p, r)) => (p, Some(r)),
            None => (entry.as_str(), None),
        };
        let anywhere = pattern.strip_prefix('~');
        let matched = match (anywhere, restore) {
            (Some(text), _) => line().is_some_and(|l| l.contains(text)),
            (None, Some(_)) => line().is_some_and(|l| l.starts_with(pattern)),
            (None, None) => pattern == name,
        };
        if !matched {
            continue;
        }
        match (restore, anywhere) {
            (Some(cmd), _) => {
                let words = tmxr_command::tokenize(cmd, &|_| None)
                    .ok()
                    .and_then(|cmds| cmds.into_iter().next())
                    .unwrap_or_default();
                if let Some((program, args)) = words.split_first() {
                    return Some((program.clone(), args.to_vec()));
                }
            }
            (None, Some(_)) => {
                let (program, args) = argv().as_ref()?.split_first()?;
                return Some((program_name(program).to_owned(), args.to_vec()));
            }
            (None, None) => {
                let args = saved_args(srv, pane, &name);
                return Some((name, args));
            }
        }
    }
    None
}

/// A program as `argv[0]` may give it (`/usr/bin/less`, `C:\…\hjkl.exe`) by
/// the name `pane_current_command` reports.
fn program_name(program: &str) -> &str {
    let base = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let len = base.len();
    match base.get(len.saturating_sub(4)..) {
        Some(ext) if len > 4 && ext.eq_ignore_ascii_case(".exe") => &base[..len - 4],
        _ => base,
    }
}

/// Capture the server's sessions.
pub fn capture(srv: &Server) -> Save {
    // Each window's first place in the save, for later sessions it is linked
    // into.
    let mut first_seen: std::collections::HashMap<WindowId, SavedLink> =
        std::collections::HashMap::new();
    let sessions = srv
        .sessions
        .values()
        .map(|s| SavedSession {
            name: s.name.clone(),
            cwd: s.cwd.clone(),
            current: s.current,
            last: s.last,
            windows: s
                .windows
                .iter()
                .filter_map(|(idx, wid)| {
                    let w = srv.windows.get(wid)?;
                    if let Some(link) = first_seen.get(wid) {
                        return Some(SavedWindow {
                            index: *idx,
                            name: w.name.clone(),
                            auto_name: w.auto_name,
                            zoomed: false,
                            synchronize: false,
                            active: 0,
                            last: None,
                            layout: SavedLayout::Leaf(0),
                            panes: Vec::new(),
                            link: Some(link.clone()),
                            manual_size: None,
                        });
                    }
                    first_seen.insert(
                        *wid,
                        SavedLink {
                            session: s.name.clone(),
                            index: *idx,
                        },
                    );
                    let panes = w.panes();
                    Some(SavedWindow {
                        index: *idx,
                        name: w.name.clone(),
                        auto_name: w.auto_name,
                        zoomed: w.zoomed,
                        synchronize: w.synchronize,
                        active: panes.iter().position(|p| *p == w.active).unwrap_or(0),
                        last: w.last_pane.and_then(|l| panes.iter().position(|p| *p == l)),
                        layout: save_layout(&w.layout, &panes),
                        panes: panes
                            .iter()
                            .map(|p| {
                                let (command, args) = saved_program(srv, *p)
                                    .map_or((None, Vec::new()), |(c, a)| (Some(c), a));
                                SavedPane {
                                    cwd: crate::vars::pane_current_path(srv, *p)
                                        .unwrap_or_default(),
                                    command,
                                    args,
                                    title: srv
                                        .panes
                                        .get(p)
                                        .and_then(|pane| pane.emu.title())
                                        .map(str::to_owned),
                                }
                            })
                            .collect(),
                        link: None,
                        manual_size: w.manual_size.then_some((w.cols, w.rows)),
                    })
                })
                .collect(),
        })
        .collect();
    Save {
        version: VERSION,
        sessions,
    }
}

/// Write a save to `dir`, update `last`, prune old saves.
pub fn write(save: &Save, dir: &Path, keep: usize) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let name = format!("tmxr-{stamp}.json");
    let path = dir.join(&name);
    let json = serde_json::to_string_pretty(save).map_err(|e| e.to_string())?;
    write_replace(&path, json.as_bytes())?;
    write_replace(&dir.join("last"), name.as_bytes())?;
    let mut saves: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("tmxr-") && n.ends_with(".json"))
        })
        .collect();
    saves.sort();
    let excess = saves.len().saturating_sub(keep.max(1));
    for old in &saves[..excess] {
        let _ = std::fs::remove_file(old);
    }
    Ok(path)
}

/// Write via a temporary file and rename, so a reader never sees a torn file.
fn write_replace(path: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Read the newest save, if any.
pub fn read_last(dir: &Path) -> Result<Option<Save>, String> {
    let Ok(name) = std::fs::read_to_string(dir.join("last")) else {
        return Ok(None);
    };
    let path = dir.join(name.trim());
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let save: Save = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if save.version != VERSION {
        return Err(format!(
            "{}: unsupported version {}",
            path.display(),
            save.version
        ));
    }
    Ok(Some(save))
}

/// `resurrect-save`: write the sessions now.
pub fn save(srv: &mut Server) -> Result<PathBuf, String> {
    let s = capture(srv);
    let path = write(&s, &dir(&srv.endpoint)?, srv.cfg.resurrect.keep)?;
    srv.last_saved = Some(s);
    Ok(path)
}

/// Auto-save and the save on exit: write only when the sessions changed
/// since the last save, so an idle server does not churn out copies that
/// push real history past `resurrect.keep`.
pub fn save_if_changed(srv: &mut Server) -> Result<Option<PathBuf>, String> {
    if srv.last_saved.as_ref() == Some(&capture(srv)) {
        return Ok(None);
    }
    save(srv).map(Some)
}

/// What a restore did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Restored {
    pub sessions: usize,
    /// Saved directories that are gone; their panes started in `$HOME`.
    pub missing_dirs: Vec<PathBuf>,
    /// Linked windows whose first session was not restored (one of that
    /// name already ran).
    pub unlinked: usize,
}

impl Restored {
    /// One line for the status line / message log.
    pub fn message(&self) -> String {
        let mut m = format!("restored {} sessions", self.sessions);
        if !self.missing_dirs.is_empty() {
            let dirs: Vec<String> = self
                .missing_dirs
                .iter()
                .map(|d| d.display().to_string())
                .collect();
            let _ = write!(m, "; missing, started in $HOME: {}", dirs.join(", "));
        }
        if self.unlinked > 0 {
            let _ = write!(
                m,
                "; {} linked windows not restored (their first session already ran)",
                self.unlinked
            );
        }
        m
    }
}

/// Recreate saved sessions whose names are not already in use.
pub fn apply(srv: &mut Server, save: &Save, size: (u16, u16)) -> Result<Restored, String> {
    let mut done = Restored::default();
    let mut placed = Placed::default();
    let mut current = Vec::new();
    for s in &save.sessions {
        if srv.sessions.values().any(|x| x.name == s.name) || s.windows.is_empty() {
            continue;
        }
        let sid = restore_session(srv, s, size, &mut done.missing_dirs, &mut placed)?;
        current.push((sid, s.current, s.last));
        done.sessions += 1;
    }
    // Linked windows, now that every window they name exists.
    for (sid, index, link) in placed.links {
        let Some(&wid) = placed.windows.get(&(link.session, link.index)) else {
            done.unlinked += 1;
            continue;
        };
        // A placeholder holds the index when the link was the session's
        // first window, which a session cannot be made without.
        let placeholder = srv
            .sessions
            .get_mut(&sid)
            .and_then(|s| s.windows.remove(&index));
        srv.link_window(wid, sid, Some(index), false)?;
        if let Some(old) = placeholder
            && srv.session_of_window(old).is_none()
        {
            srv.kill_window(old);
        }
    }
    // Last, as the current window may be a link.
    for (sid, cur, last) in current {
        if let Some(sess) = srv.sessions.get_mut(&sid) {
            if sess.windows.contains_key(&cur) {
                sess.current = cur;
            }
            sess.last = last.filter(|l| sess.windows.contains_key(l));
        }
    }
    done.missing_dirs.sort();
    done.missing_dirs.dedup();
    Ok(done)
}

/// What a restore has placed so far, for the links made at its end.
#[derive(Default)]
struct Placed {
    /// (session name, index) → the window restored there.
    windows: std::collections::HashMap<(String, u32), WindowId>,
    /// Links still to make: session, index, the window's first place.
    links: Vec<(SessionId, u32, SavedLink)>,
}

/// `p` if it is still a directory, else `$HOME`, noting `p` in `missing`.
fn existing_dir(p: &Path, missing: &mut Vec<PathBuf>) -> PathBuf {
    if p.is_dir() {
        p.to_path_buf()
    } else {
        missing.push(p.to_path_buf());
        crate::util::home_dir()
    }
}

fn restore_session(
    srv: &mut Server,
    s: &SavedSession,
    size: (u16, u16),
    missing: &mut Vec<PathBuf>,
    placed: &mut Placed,
) -> Result<SessionId, String> {
    let first = &s.windows[0];
    let argv = |p: &SavedPane| {
        p.command
            .iter()
            .chain(p.command.is_some().then_some(&p.args).into_iter().flatten())
            .cloned()
            .collect::<Vec<_>>()
    };
    let first_pane = first.panes.first().cloned().unwrap_or_else(|| SavedPane {
        cwd: s.cwd.clone(),
        command: None,
        args: Vec::new(),
        title: None,
    });
    let sid = srv.new_session(
        Some(s.name.clone()),
        existing_dir(&s.cwd, missing),
        Vec::new(),
        (!first.auto_name).then(|| first.name.clone()),
        argv(&first_pane),
        size,
    )?;
    // new_session put the first window at base-index; move it to its saved
    // index.
    if let Some(sess) = srv.sessions.get_mut(&sid)
        && let Some((_, wid)) = sess.windows.pop_first()
    {
        sess.windows.insert(first.index, wid);
        sess.current = first.index;
    }
    for (n, w) in s.windows.iter().enumerate() {
        if let Some(link) = &w.link {
            placed.links.push((sid, w.index, link.clone()));
            continue;
        }
        let wid = if n == 0 {
            srv.sessions[&sid].windows[&w.index]
        } else {
            let p0 = w.panes.first().cloned().unwrap_or_else(|| SavedPane {
                cwd: s.cwd.clone(),
                command: None,
                args: Vec::new(),
                title: None,
            });
            srv.new_window(
                sid,
                Some(w.index),
                (!w.auto_name).then(|| w.name.clone()),
                Some(existing_dir(&p0.cwd, missing)),
                argv(&p0),
                &[],
                size,
                false,
            )?
        };
        // The remaining panes, split off the first one; the saved shape is
        // applied once they all exist.
        let mut panes = srv.windows[&wid].panes();
        for p in w.panes.iter().skip(1) {
            let last = *panes.last().unwrap_or(&0);
            let id = srv.split(
                last,
                true,
                false,
                None,
                existing_dir(&p.cwd, missing),
                argv(p),
                &[],
                false,
            )?;
            panes.push(id);
        }
        for (id, saved) in panes.iter().zip(&w.panes) {
            if let (Some(p), Some(title)) = (srv.panes.get_mut(id), &saved.title) {
                p.emu.set_title(title.clone());
            }
        }
        if let Some(win) = srv.windows.get_mut(&wid) {
            if let Some(tree) = load_layout(&w.layout, &panes) {
                win.layout = tree;
            }
            win.active = panes.get(w.active).copied().unwrap_or(win.active);
            win.last_pane = w.last.and_then(|l| panes.get(l).copied());
            win.zoomed = w.zoomed && panes.len() > 1;
            win.synchronize = w.synchronize;
            win.auto_name = w.auto_name;
            if let Some((cols, rows)) = w.manual_size {
                win.manual_size = true;
                win.cols = cols;
                win.rows = rows;
            }
            if !w.auto_name {
                win.name.clone_from(&w.name);
            }
        }
        srv.relayout(wid);
        placed.windows.insert((s.name.clone(), w.index), wid);
    }
    Ok(sid)
}

/// `resurrect-restore`.
pub fn restore(srv: &mut Server, size: (u16, u16)) -> Result<Restored, String> {
    match read_last(&dir(&srv.endpoint)?)? {
        Some(save) => apply(srv, &save, size),
        None => Err("no saved sessions".into()),
    }
}

/// Restore when the server starts (`resurrect.restore-on-start`).
pub fn restore_on_start(srv: &mut Server) -> Result<(), String> {
    if let Some(save) = read_last(&dir(&srv.endpoint)?)? {
        let done = apply(srv, &save, (80, 23))?;
        srv.restored_pending = done.sessions > 0;
        if !done.missing_dirs.is_empty() {
            srv.log_message(format!("resurrect: {}", done.message()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Save {
        Save {
            version: VERSION,
            sessions: vec![SavedSession {
                name: "main".into(),
                cwd: "/home/u".into(),
                current: 1,
                last: Some(0),
                windows: vec![SavedWindow {
                    index: 1,
                    name: "hjkl".into(),
                    auto_name: true,
                    zoomed: false,
                    synchronize: true,
                    active: 1,
                    last: Some(0),
                    layout: SavedLayout::Split {
                        horizontal: true,
                        ratio: 0.3,
                        a: Box::new(SavedLayout::Leaf(0)),
                        b: Box::new(SavedLayout::Leaf(1)),
                    },
                    panes: vec![
                        SavedPane {
                            cwd: "/home/u/src".into(),
                            command: Some("hjkl".into()),
                            args: vec!["notes.md".into()],
                            title: None,
                        },
                        SavedPane {
                            cwd: "/tmp".into(),
                            command: None,
                            args: Vec::new(),
                            title: None,
                        },
                    ],
                    link: None,
                    manual_size: None,
                }],
            }],
        }
    }

    #[test]
    fn layout_round_trips_through_pane_positions() {
        let tree = LayoutTree::split(
            SplitDir::Horizontal,
            0.25,
            LayoutTree::Leaf(7),
            LayoutTree::split(
                SplitDir::Vertical,
                0.5,
                LayoutTree::Leaf(3),
                LayoutTree::Leaf(9),
            ),
        );
        let saved = save_layout(&tree, &[7, 3, 9]);
        let back = load_layout(&saved, &[10, 11, 12]).unwrap();
        assert_eq!(back.leaves(), vec![10, 11, 12]);
        let rects = |t: &LayoutTree| t.window_rects(hjkl_layout::LayoutRect::new(0, 0, 80, 24));
        let a: Vec<_> = rects(&tree).into_iter().map(|(_, r)| r).collect();
        let b: Vec<_> = rects(&back).into_iter().map(|(_, r)| r).collect();
        assert_eq!(a, b);
        assert!(
            load_layout(&saved, &[1]).is_none(),
            "missing pane is an error"
        );
    }

    #[test]
    fn saves_without_last_pane_still_load() {
        let mut json: serde_json::Value = serde_json::to_value(sample()).unwrap();
        let window = &mut json["sessions"][0]["windows"][0];
        assert!(window.as_object_mut().unwrap().remove("last").is_some());
        let old: Save = serde_json::from_value(json).unwrap();
        assert_eq!(old.sessions[0].windows[0].last, None);
    }

    #[test]
    fn saves_without_args_still_load() {
        let mut json: serde_json::Value = serde_json::to_value(sample()).unwrap();
        let panes = &mut json["sessions"][0]["windows"][0]["panes"];
        assert!(panes[1].get("args").is_none(), "empty args are not written");
        assert!(panes[0].as_object_mut().unwrap().remove("args").is_some());
        let old: Save = serde_json::from_value(json).unwrap();
        assert!(old.sessions[0].windows[0].panes[0].args.is_empty());
    }

    #[test]
    fn program_names_drop_their_directory_and_exe_suffix() {
        assert_eq!(program_name("/usr/bin/less"), "less");
        assert_eq!(program_name(r"C:\Tools\hjkl.exe"), "hjkl");
        assert_eq!(program_name("PING.EXE"), "PING");
        assert_eq!(program_name("nvim"), "nvim");
        assert_eq!(program_name(".exe"), ".exe");
    }

    #[test]
    fn write_read_and_prune() {
        let dir = tempfile::tempdir().unwrap();
        let s = sample();
        for _ in 0..4 {
            write(&s, dir.path(), 2).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(read_last(dir.path()).unwrap(), Some(s));
        let saves = std::fs::read_dir(dir.path())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".json")
            })
            .count();
        assert_eq!(saves, 2);
        assert_eq!(read_last(&dir.path().join("nothing-here")).unwrap(), None);
    }
}
