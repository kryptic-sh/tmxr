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

use crate::model::{PaneId, SessionId};
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedPane {
    pub cwd: PathBuf,
    /// Program to start again, when it is in `resurrect.processes`.
    pub command: Option<String>,
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

/// Capture the server's sessions.
pub fn capture(srv: &Server) -> Save {
    let keep = &srv.cfg.resurrect.processes;
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
                            .map(|p| SavedPane {
                                cwd: crate::vars::pane_current_path(srv, *p).unwrap_or_default(),
                                command: srv.commands.get(p).filter(|c| keep.contains(c)).cloned(),
                            })
                            .collect(),
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
        m
    }
}

/// Recreate saved sessions whose names are not already in use.
pub fn apply(srv: &mut Server, save: &Save, size: (u16, u16)) -> Result<Restored, String> {
    let mut done = Restored::default();
    for s in &save.sessions {
        if srv.sessions.values().any(|x| x.name == s.name) || s.windows.is_empty() {
            continue;
        }
        restore_session(srv, s, size, &mut done.missing_dirs)?;
        done.sessions += 1;
    }
    done.missing_dirs.sort();
    done.missing_dirs.dedup();
    Ok(done)
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
) -> Result<SessionId, String> {
    let first = &s.windows[0];
    let argv = |p: &SavedPane| p.command.clone().map(|c| vec![c]).unwrap_or_default();
    let first_pane = first.panes.first().cloned().unwrap_or_else(|| SavedPane {
        cwd: s.cwd.clone(),
        command: None,
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
        let wid = if n == 0 {
            srv.sessions[&sid].windows[&w.index]
        } else {
            let p0 = w.panes.first().cloned().unwrap_or_else(|| SavedPane {
                cwd: s.cwd.clone(),
                command: None,
            });
            srv.new_window(
                sid,
                Some(w.index),
                (!w.auto_name).then(|| w.name.clone()),
                Some(existing_dir(&p0.cwd, missing)),
                argv(&p0),
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
                false,
            )?;
            panes.push(id);
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
            if !w.auto_name {
                win.name.clone_from(&w.name);
            }
        }
        srv.relayout(wid);
    }
    if let Some(sess) = srv.sessions.get_mut(&sid) {
        if sess.windows.contains_key(&s.current) {
            sess.current = s.current;
        }
        sess.last = s.last.filter(|l| sess.windows.contains_key(l));
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
                        },
                        SavedPane {
                            cwd: "/tmp".into(),
                            command: None,
                        },
                    ],
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
