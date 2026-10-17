//! Things drawn over a client's panes that take its keys while open: the
//! command prompt, y/n confirmation, scrollable text (list-keys output),
//! the session / window picker and the `display-panes` numbers.

mod boxes;
mod picker;
mod prompt;

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::model::{ClientId, PaneId, SessionId};
use crate::server::Server;

pub use boxes::{BoxLook, Lines};
use picker::{Item, Target, current_first};
pub use picker::{PickerOverlay, Previewed};
pub use prompt::Prompt;

/// What a key did to an overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayAction {
    Keep,
    Close,
    /// Close and run this command list.
    Run(String),
    /// Close and switch the client to this session.
    Switch(SessionId),
    /// Close, switch to the session and select the window index.
    SwitchWindow(SessionId, u32),
    /// Run this command list and keep the overlay open
    /// (`command-prompt -i`, after each edit).
    Preview(String),
    /// Close and open the command prompt holding this command line, to be
    /// edited and run (`customize-mode`).
    Edit(String),
}

pub enum Overlay {
    /// `display-popup`.
    Popup(Box<crate::popup::Popup>),
    /// `display-menu`.
    Menu(Box<crate::menu::Menu>),
    Prompt(Prompt),
    Confirm {
        prompt: String,
        cmd: String,
    },
    Text {
        lines: Vec<String>,
        top: usize,
    },
    Picker(Box<PickerOverlay>),
    /// `display-panes`: each pane's number, shown until `until`.
    Panes {
        labels: Vec<(u32, PaneId)>,
        until: Instant,
    },
}

impl Overlay {
    pub fn prompt(prompt: String, initial: String, template: Option<String>) -> Self {
        let input: Vec<char> = initial.chars().collect();
        let cursor = input.len();
        Self::Prompt(Prompt {
            prompt,
            input,
            cursor,
            template,
            key: false,
            incremental: false,
            kind: "command".into(),
            history: Vec::new(),
            recall: None,
        })
    }

    /// A prompt answered by one key press (`command-prompt -k`).
    pub fn key_prompt(prompt: String, template: String) -> Self {
        Self::Prompt(Prompt {
            prompt,
            input: Vec::new(),
            cursor: 0,
            template: Some(template),
            key: true,
            incremental: false,
            kind: "command".into(),
            history: Vec::new(),
            recall: None,
        })
    }

    pub fn confirm(prompt: String, cmd: String) -> Self {
        Self::Confirm { prompt, cmd }
    }

    pub fn text(lines: Vec<String>) -> Self {
        Self::Text { lines, top: 0 }
    }

    /// Show `labels` (pane number, pane) for `time`.
    pub fn display_panes(labels: Vec<(u32, PaneId)>, time: Duration) -> Self {
        Self::Panes {
            labels,
            until: Instant::now() + time,
        }
    }

    /// Whether the overlay has timed out and should close by itself.
    pub fn expired(&self) -> bool {
        matches!(self, Self::Panes { until, .. } if *until <= Instant::now())
    }

    pub fn session_picker(srv: &Server, client: ClientId) -> Self {
        let current = srv
            .clients
            .get(&client)
            .and_then(|c| c.att.as_ref())
            .map(|a| a.session);
        let mut sessions: Vec<_> = srv.sessions.values().collect();
        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_used));
        current_first(&mut sessions, |s| Some(s.id) == current);
        let items = sessions
            .iter()
            .map(|s| {
                let attached = srv
                    .clients
                    .values()
                    .any(|c| c.att.as_ref().is_some_and(|a| a.session == s.id));
                let mark = if Some(s.id) == current { "*" } else { " " };
                Item {
                    label: format!(
                        "{mark} {}: {} windows{}",
                        s.name,
                        s.windows.len(),
                        if attached { " (attached)" } else { "" }
                    ),
                    matches: s.name.clone(),
                    target: Target::Session(s.id, s.name.clone()),
                }
            })
            .collect();
        Self::Picker(Box::new(PickerOverlay::new("sessions", items)))
    }

    /// Every session's windows, the client's session first.
    pub fn window_picker(srv: &Server, client: ClientId) -> Self {
        Self::found_windows(srv, client, |_| true)
    }

    /// The windows `keep` accepts, laid out as [`Overlay::window_picker`]
    /// (`find-window`).
    pub fn found_windows(
        srv: &Server,
        client: ClientId,
        keep: impl Fn(&crate::model::Window) -> bool,
    ) -> Self {
        let current = srv
            .clients
            .get(&client)
            .and_then(|c| c.att.as_ref())
            .map(|a| a.session);
        let mut items = Vec::new();
        let mut sessions: Vec<_> = srv.sessions.values().collect();
        sessions.sort_by_key(|s| (Some(s.id) != current, s.name.clone()));
        for s in sessions {
            for (idx, wid) in &s.windows {
                let Some(w) = srv.windows.get(wid).filter(|w| keep(w)) else {
                    continue;
                };
                let mark = if Some(s.id) == current && *idx == s.current {
                    "*"
                } else {
                    " "
                };
                items.push(Item {
                    label: format!("{mark} {}:{idx} {}", s.name, w.name),
                    matches: format!("{}:{idx} {}", s.name, w.name),
                    target: Target::Window(s.id, *idx),
                });
            }
        }
        Self::Picker(Box::new(PickerOverlay::new("windows", items)))
    }

    /// The attached clients; Enter detaches one (`choose-client`).
    pub fn client_picker(srv: &Server) -> Self {
        let items = srv
            .clients
            .values()
            .filter_map(|c| {
                let att = c.att.as_ref()?;
                let session = srv
                    .sessions
                    .get(&att.session)
                    .map_or("", |s| s.name.as_str());
                Some(Item {
                    label: format!("{}: {session} [{}x{}]", c.id, att.cols, att.rows),
                    matches: format!("{} {session}", c.id),
                    target: Target::Client(c.id),
                })
            })
            .collect();
        Self::Picker(Box::new(PickerOverlay::new("clients", items)))
    }

    /// The paste buffers, newest first; Enter pastes one into the pane.
    pub fn buffer_picker(srv: &Server) -> Self {
        let items = srv
            .buffers
            .iter()
            .map(|b| {
                let preview: String = b
                    .data
                    .chars()
                    .take(50)
                    .map(|c| if c.is_control() { ' ' } else { c })
                    .collect();
                Item {
                    label: format!("{}: {} bytes: \"{preview}\"", b.name, b.data.len()),
                    matches: format!("{} {preview}", b.name),
                    target: Target::Buffer(b.name.clone()),
                }
            })
            .collect();
        Self::Picker(Box::new(PickerOverlay::new("buffers", items)))
    }

    /// `customize-mode`: every option and key bind, as the command that sets
    /// it; Enter puts that command in the prompt to change. Opens filtered
    /// by `query`.
    pub fn customize_picker(srv: &Server, query: &str) -> Self {
        let options = crate::cmds::option_lines(srv).into_iter().map(|l| Item {
            label: format!("option {l}"),
            matches: l.clone(),
            target: Target::Edit(format!("set-option -g {l}")),
        });
        let binds = srv.keys.list(None).into_iter().map(|(table, key, b)| {
            let line = crate::cmds::bind_line(&table, &key, &b);
            Item {
                label: line.clone(),
                matches: line.clone(),
                target: Target::Edit(line),
            }
        });
        let items = options.chain(binds).collect();
        Self::Picker(Box::new(PickerOverlay::with_query(
            "customize",
            items,
            query,
        )))
    }

    /// Feed a key. Returns what the server should do.
    pub fn key(&mut self, ev: &KeyEvent) -> OverlayAction {
        let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
        match self {
            Self::Prompt(p) => p.key(ev),
            Self::Menu(m) => m.key(ev),
            Self::Popup(p) => p.key(ev),
            Self::Confirm { cmd, .. } => match ev.code {
                KeyCode::Char('y' | 'Y') => OverlayAction::Run(cmd.clone()),
                _ => OverlayAction::Close,
            },
            Self::Text { lines, top } => {
                let max = lines.len().saturating_sub(1);
                match ev.code {
                    KeyCode::Char('q') | KeyCode::Esc => return OverlayAction::Close,
                    KeyCode::Char('c') if ctrl => return OverlayAction::Close,
                    KeyCode::Char('j') | KeyCode::Down => *top = (*top + 1).min(max),
                    KeyCode::Char('k') | KeyCode::Up => *top = top.saturating_sub(1),
                    KeyCode::Char('d') if ctrl => *top = (*top + 10).min(max),
                    KeyCode::Char('u') if ctrl => *top = top.saturating_sub(10),
                    KeyCode::PageDown | KeyCode::Char(' ') => *top = (*top + 20).min(max),
                    KeyCode::PageUp => *top = top.saturating_sub(20),
                    KeyCode::Char('g') | KeyCode::Home => *top = 0,
                    KeyCode::Char('G') | KeyCode::End => *top = max,
                    _ => {}
                }
                OverlayAction::Keep
            }
            Self::Picker(p) => p.key(ev),
            // A pane's number selects it; any other key just dismisses.
            Self::Panes { labels, .. } => {
                let pick = match ev.code {
                    KeyCode::Char(c) => c.to_digit(10).and_then(|d| {
                        labels
                            .iter()
                            .find(|(label, _)| *label == d)
                            .map(|(_, p)| *p)
                    }),
                    _ => None,
                };
                pick.map_or(OverlayAction::Close, |p| {
                    OverlayAction::Run(format!("select-pane -t %{p}"))
                })
            }
        }
    }

    pub fn paste(&mut self, text: &str) {
        match self {
            Self::Prompt(p) => {
                for c in text.chars().filter(|c| !c.is_control()) {
                    p.input.insert(p.cursor, c);
                    p.cursor += 1;
                }
            }
            Self::Popup(p) => p.paste(text),
            _ => {}
        }
    }

    /// Periodic work; returns whether a redraw is needed.
    pub fn tick(&mut self) -> bool {
        match self {
            Self::Picker(p) => {
                p.picker.tick(Instant::now());
                p.picker.refresh()
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn items(names: &[&str]) -> Vec<Item> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| Item {
                label: (*n).to_owned(),
                matches: (*n).to_owned(),
                target: Target::Session(i as u32, (*n).to_owned()),
            })
            .collect()
    }

    #[test]
    fn prompt_edits_and_substitutes_the_template() {
        let mut o = Overlay::prompt(":".into(), "ab".into(), Some("rename-window -- %%".into()));
        o.key(&k(KeyCode::Backspace));
        o.key(&k(KeyCode::Char('x')));
        o.key(&k(KeyCode::Home));
        o.key(&k(KeyCode::Char('>')));
        assert_eq!(
            o.key(&k(KeyCode::Enter)),
            OverlayAction::Run("rename-window -- >ax".into())
        );
        let mut o = Overlay::prompt(":".into(), String::new(), None);
        assert_eq!(o.key(&k(KeyCode::Esc)), OverlayAction::Close);
    }

    #[test]
    fn confirm_runs_only_on_y() {
        let mut o = Overlay::confirm("kill?".into(), "kill-pane".into());
        assert_eq!(
            o.key(&k(KeyCode::Char('y'))),
            OverlayAction::Run("kill-pane".into())
        );
        let mut o = Overlay::confirm("kill?".into(), "kill-pane".into());
        assert_eq!(o.key(&k(KeyCode::Char('n'))), OverlayAction::Close);
    }

    #[test]
    fn session_picker_opens_on_the_previous_session() {
        // MRU order: the current session was used last, "work" before it.
        let mut mru = vec!["main", "work", "dots"];
        current_first(&mut mru, |s| *s == "main");
        assert_eq!(mru, ["main", "work", "dots"]);
        let o = PickerOverlay::new("sessions", items(&mru));
        assert_eq!(mru[o.picker.selected], "work");
        // The current session is not always the most recent (another client
        // switched since); it still goes first and keeps the rest in order.
        let mut mru = vec!["work", "dots", "main"];
        current_first(&mut mru, |s| *s == "main");
        assert_eq!(mru, ["main", "work", "dots"]);
    }

    #[test]
    fn prompt_history_recalls_older_and_newer_and_the_draft() {
        let mut o = Overlay::prompt(":".into(), String::new(), None);
        let Overlay::Prompt(p) = &mut o else {
            unreachable!()
        };
        p.history = vec!["first".into(), "second".into()];
        o.key(&k(KeyCode::Char('x')));
        let text = |o: &Overlay| match o {
            Overlay::Prompt(p) => p.text(),
            _ => unreachable!(),
        };
        o.key(&k(KeyCode::Up));
        assert_eq!(text(&o), "second");
        o.key(&k(KeyCode::Up));
        assert_eq!(text(&o), "first");
        o.key(&k(KeyCode::Up));
        assert_eq!(text(&o), "first", "the oldest stays");
        o.key(&k(KeyCode::Down));
        assert_eq!(text(&o), "second");
        o.key(&k(KeyCode::Down));
        assert_eq!(text(&o), "x", "the typed draft comes back");
    }

    #[test]
    fn picker_filters_moves_with_jk_and_selects() {
        let sessions = || {
            Overlay::Picker(Box::new(PickerOverlay::new(
                "sessions",
                items(&["main", "work", "dots"]),
            )))
        };
        let query = |o: &Overlay| {
            let Overlay::Picker(p) = o else {
                unreachable!()
            };
            (p.query(), p.picker.matched(), p.typing())
        };
        // Sessions open on the second entry (the previous session), in
        // normal mode.
        let mut o = sessions();
        let Overlay::Picker(p) = &o else {
            unreachable!()
        };
        assert_eq!(p.picker.selected, 1);
        assert!(!p.typing());
        // Letters are not typed until `i`; then typing filters.
        o.key(&k(KeyCode::Char('d')));
        assert_eq!(query(&o), (String::new(), 3, false));
        o.key(&k(KeyCode::Char('i')));
        o.key(&k(KeyCode::Char('d')));
        o.key(&k(KeyCode::Char('o')));
        assert_eq!(query(&o), ("do".to_owned(), 1, true));
        assert_eq!(o.key(&k(KeyCode::Enter)), OverlayAction::Switch(2));

        // j and k move in normal mode, and so do arrows and C-n / C-p.
        let mut o = sessions();
        o.key(&k(KeyCode::Char('j')));
        assert_eq!(o.key(&k(KeyCode::Enter)), OverlayAction::Switch(2));
        let mut o = sessions();
        o.key(&k(KeyCode::Char('k')));
        assert_eq!(o.key(&k(KeyCode::Enter)), OverlayAction::Switch(0));
        let mut o = sessions();
        o.key(&k(KeyCode::Down));
        assert_eq!(o.key(&k(KeyCode::Enter)), OverlayAction::Switch(2));
        let mut o = sessions();
        let ctrl_p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        o.key(&ctrl_p);
        o.key(&ctrl_p);
        assert_eq!(
            o.key(&k(KeyCode::Enter)),
            OverlayAction::Switch(2),
            "C-p wraps"
        );

        // While typing, j is typed and Escape goes back to normal mode,
        // keeping the query; another Escape closes.
        let mut o = sessions();
        o.key(&k(KeyCode::Char('/')));
        o.key(&k(KeyCode::Char('j')));
        assert_eq!(query(&o), ("j".to_owned(), 0, true));
        assert_eq!(o.key(&k(KeyCode::Esc)), OverlayAction::Keep);
        assert_eq!(query(&o), ("j".to_owned(), 0, false));
        assert_eq!(o.key(&k(KeyCode::Esc)), OverlayAction::Close);
        assert_eq!(sessions().key(&k(KeyCode::Char('q'))), OverlayAction::Close);
    }
}
