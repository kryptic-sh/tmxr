//! Things drawn over a client's panes that take its keys while open: the
//! command prompt, y/n confirmation, scrollable text (list-keys output),
//! the session / window picker and the `display-panes` numbers.

mod boxes;
mod picker;
mod prompt;

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::model::{ClientId, PaneId, SessionId, WindowId};
use crate::server::Server;
use crate::vars::Vars;

pub use boxes::{BoxLook, Lines};
use picker::{Item, Target, current_first};
pub use picker::{PickerOverlay, Previewed};
pub use prompt::Prompt;

/// The orders `-O` names, for the `choose-*` pickers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Index,
    Name,
    /// The most recently active (a buffer: made) first.
    Time,
    /// The smallest client, the largest buffer, first.
    Size,
    /// The newest client first.
    Creation,
    /// The client last typed into first.
    Activity,
}

/// The orders each picker takes, as tmux's window_tree_sort_list,
/// window_client_sort_list and window_buffer_sort_list.
const TREE_SORTS: &[(&str, Sort)] = &[
    ("index", Sort::Index),
    ("name", Sort::Name),
    ("time", Sort::Time),
];
const CLIENT_SORTS: &[(&str, Sort)] = &[
    ("name", Sort::Name),
    ("size", Sort::Size),
    ("creation", Sort::Creation),
    ("activity", Sort::Activity),
];
const BUFFER_SORTS: &[(&str, Sort)] = &[
    ("time", Sort::Time),
    ("name", Sort::Name),
    ("size", Sort::Size),
];

/// The `choose-*` pickers' -F, -f, -O and -r, as tmux's: each item's text,
/// the filter (in `choose-tree` a session or window stays while any of its
/// panes passes, as tmux's tree keeps them), the order and whether it is
/// turned round. Without -O the picker keeps its own order.
#[derive(Debug, Default)]
pub struct Choose<'a> {
    pub format: Option<&'a str>,
    pub filter: Option<&'a str>,
    /// -O as given; one tmux does not know leaves the order alone, as there.
    pub sort: Option<&'a str>,
    pub reverse: bool,
    /// The client choosing, for the formats' `client_*`.
    pub client: Option<ClientId>,
}

impl Sort {
    /// `sessions` in this order, as tmux's window_tree_cmp_session: by id,
    /// by name, or the most recently active first and then by name.
    fn sessions(self, sessions: &mut [&crate::model::Session]) {
        match self {
            Self::Index => sessions.sort_by_key(|s| s.id),
            Self::Name | Self::Size | Self::Creation | Self::Activity => {
                sessions.sort_by(|a, b| a.name.cmp(&b.name));
            }
            Self::Time => {
                sessions.sort_by(|a, b| {
                    b.activity
                        .cmp(&a.activity)
                        .then_with(|| a.name.cmp(&b.name))
                });
            }
        }
    }
}

impl Choose<'_> {
    /// The order -O names among those a picker takes.
    fn sort(&self, takes: &[(&str, Sort)]) -> Option<Sort> {
        let sort = self.sort?;
        takes
            .iter()
            .find(|(name, _)| sort.eq_ignore_ascii_case(name))
            .map(|(_, s)| *s)
    }

    /// Whether the tree is not in the picker's own order, so it opens on
    /// the current item.
    fn reorders(&self) -> bool {
        self.sort(TREE_SORTS).is_some() || self.reverse
    }

    /// Whether the filter passes in `vars`.
    fn passes(&self, vars: &dyn tmxr_command::format::Context) -> bool {
        self.filter
            .is_none_or(|f| crate::cmds::format_true(&tmxr_command::format::expand(f, vars)))
    }

    /// Whether any pane of `windows` (in `session`) passes the filter.
    fn keeps(
        &self,
        srv: &Server,
        session: SessionId,
        windows: impl Iterator<Item = WindowId>,
    ) -> bool {
        let Some(filter) = self.filter else {
            return true;
        };
        windows
            .filter_map(|w| srv.windows.get(&w))
            .flat_map(|w| w.panes().into_iter().map(move |p| (w.id, p)))
            .any(|(w, p)| {
                let vars = Vars {
                    srv,
                    session: Some(session),
                    window: Some(w),
                    pane: Some(p),
                    client: self.client,
                };
                crate::cmds::format_true(&tmxr_command::format::expand(filter, &vars))
            })
    }

    /// An item's text: -F expanded for it, else `default`.
    fn text(
        &self,
        vars: &dyn tmxr_command::format::Context,
        default: impl FnOnce() -> String,
    ) -> String {
        self.format
            .map_or_else(default, |f| tmxr_command::format::expand(f, vars))
    }
}

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
    /// Keep the overlay and let the key go on as if it were not there
    /// (`display-panes -N`).
    Pass,
}

pub enum Overlay {
    /// `display-popup`.
    Popup(Box<crate::popup::Popup>),
    /// `display-menu`.
    Menu(Box<crate::menu::Menu>),
    Prompt(Box<Prompt>),
    /// `confirm-before`: `cmd` runs on `key`, or on Enter with
    /// `default_yes` (`-y`); any other key closes it.
    Confirm {
        prompt: String,
        cmd: String,
        key: char,
        default_yes: bool,
    },
    Text {
        lines: Vec<String>,
        top: usize,
    },
    Picker(Box<PickerOverlay>),
    /// `display-panes`: each pane's number, shown until `until`.
    Panes {
        labels: Vec<(u32, PaneId)>,
        /// When it closes by itself; never for `-d 0`.
        until: Option<Instant>,
        /// The command list a chosen pane runs, `%%` its id.
        template: String,
        /// Keys choose a pane; with `-N` they go on as usual.
        keys: bool,
    },
}

impl Overlay {
    pub fn prompt(prompt: String, initial: String, template: Option<String>) -> Self {
        let input: Vec<char> = initial.chars().collect();
        let cursor = input.len();
        Self::Prompt(Box::new(Prompt {
            prompt,
            input,
            cursor,
            template,
            pending: std::collections::VecDeque::new(),
            answers: Vec::new(),
            key: false,
            incremental: false,
            last: String::new(),
            submitted: false,
            single: false,
            numeric: false,
            kind: "command".into(),
            history: Vec::new(),
            recall: None,
            vi: false,
            command_mode: false,
            separators: String::new(),
            paste: None,
            saved: None,
            quote_next: false,
        }))
    }

    /// A prompt answered by one key press (`command-prompt -k`).
    pub fn key_prompt(prompt: String, template: String) -> Self {
        let mut o = Self::prompt(prompt, String::new(), Some(template));
        if let Self::Prompt(p) = &mut o {
            p.key = true;
        }
        o
    }

    pub fn confirm(prompt: String, cmd: String, key: char, default_yes: bool) -> Self {
        Self::Confirm {
            prompt,
            cmd,
            key,
            default_yes,
        }
    }

    pub fn text(lines: Vec<String>) -> Self {
        Self::Text { lines, top: 0 }
    }

    /// Show `labels` (pane number, pane) for `time` (until a key with
    /// zero), a chosen pane running `template`; with `keys` off, keys go on
    /// as usual instead, as tmux's `display-panes -N`.
    pub fn display_panes(
        labels: Vec<(u32, PaneId)>,
        time: Duration,
        template: String,
        keys: bool,
    ) -> Self {
        Self::Panes {
            labels,
            until: (!time.is_zero()).then(|| Instant::now() + time),
            template,
            keys,
        }
    }

    /// Whether the overlay has timed out and should close by itself.
    pub fn expired(&self) -> bool {
        matches!(self, Self::Panes { until: Some(until), .. } if *until <= Instant::now())
    }

    /// Every session, `current` (marked) first, unless `choose` sorts them.
    pub fn session_picker(srv: &Server, current: Option<SessionId>, choose: &Choose<'_>) -> Self {
        let mut sessions: Vec<_> = srv
            .sessions
            .values()
            .filter(|s| choose.keeps(srv, s.id, s.windows.values().copied()))
            .collect();
        match choose.sort(TREE_SORTS) {
            None => {
                sessions.sort_by_key(|s| std::cmp::Reverse(s.last_used));
                current_first(&mut sessions, |s| Some(s.id) == current);
            }
            Some(sort) => sort.sessions(&mut sessions),
        }
        if choose.reverse {
            sessions.reverse();
        }
        let items = sessions
            .iter()
            .map(|s| {
                let attached = srv
                    .clients
                    .values()
                    .any(|c| c.att.as_ref().is_some_and(|a| a.session == s.id));
                let mark = if Some(s.id) == current { "*" } else { " " };
                let window = s.current_window();
                let vars = Vars {
                    srv,
                    session: Some(s.id),
                    window,
                    pane: window.and_then(|w| srv.windows.get(&w)).map(|w| w.active),
                    client: choose.client,
                };
                let text = choose.text(&vars, || {
                    format!(
                        "{}: {} windows{}",
                        s.name,
                        s.windows.len(),
                        if attached { " (attached)" } else { "" }
                    )
                });
                Item {
                    label: format!("{mark} {text}"),
                    matches: s.name.clone(),
                    target: Target::Session(s.id, s.name.clone()),
                }
            })
            .collect();
        let mut picker = PickerOverlay::new("sessions", items);
        if choose.reorders() {
            picker.select(sessions.iter().position(|s| Some(s.id) == current));
        }
        Self::Picker(Box::new(picker))
    }

    /// Every session's windows, `current`'s session first and its window
    /// marked, unless `choose` sorts them.
    pub fn window_picker(
        srv: &Server,
        current: Option<(SessionId, u32)>,
        choose: &Choose<'_>,
    ) -> Self {
        Self::windows_picker(srv, current, |_| true, choose)
    }

    /// The windows `keep` accepts, laid out as [`Overlay::window_picker`]
    /// (`find-window`).
    pub fn found_windows(
        srv: &Server,
        current: Option<(SessionId, u32)>,
        keep: impl Fn(&crate::model::Window) -> bool,
    ) -> Self {
        Self::windows_picker(srv, current, keep, &Choose::default())
    }

    fn windows_picker(
        srv: &Server,
        current: Option<(SessionId, u32)>,
        keep: impl Fn(&crate::model::Window) -> bool,
        choose: &Choose<'_>,
    ) -> Self {
        let (current, current_index) = (current.map(|c| c.0), current.map(|c| c.1));
        let mut sessions: Vec<_> = srv.sessions.values().collect();
        match choose.sort(TREE_SORTS) {
            None => sessions.sort_by_key(|s| (Some(s.id) != current, s.name.clone())),
            Some(sort) => sort.sessions(&mut sessions),
        }
        let mut rows = Vec::new();
        for s in sessions {
            let mut windows: Vec<(u32, &crate::model::Window)> = s
                .windows
                .iter()
                .filter_map(|(idx, wid)| Some((*idx, srv.windows.get(wid).filter(|w| keep(w))?)))
                .filter(|(_, w)| choose.keeps(srv, s.id, std::iter::once(w.id)))
                .collect();
            let by_name = |a: &(u32, &crate::model::Window), b: &(u32, &crate::model::Window)| {
                a.1.name.cmp(&b.1.name)
            };
            match choose.sort(TREE_SORTS) {
                Some(Sort::Name) => windows.sort_by(by_name),
                Some(Sort::Time) => windows.sort_by(|a, b| {
                    b.1.last_output
                        .cmp(&a.1.last_output)
                        .then_with(|| by_name(a, b))
                }),
                _ => {}
            }
            rows.extend(windows.into_iter().map(|(idx, w)| (s, idx, w)));
        }
        if choose.reverse {
            rows.reverse();
        }
        let is_current = |s: &crate::model::Session, idx: u32| {
            Some(s.id) == current && Some(idx) == current_index
        };
        let items = rows
            .iter()
            .map(|(s, idx, w)| {
                let mark = if is_current(s, *idx) { "*" } else { " " };
                let vars = Vars {
                    srv,
                    session: Some(s.id),
                    window: Some(w.id),
                    pane: Some(w.active),
                    client: choose.client,
                };
                let text = choose.text(&vars, || format!("{}:{idx} {}", s.name, w.name));
                Item {
                    label: format!("{mark} {text}"),
                    matches: format!("{}:{idx} {}", s.name, w.name),
                    target: Target::Window(s.id, *idx, s.name.clone()),
                }
            })
            .collect();
        let mut picker = PickerOverlay::new("windows", items);
        if choose.reorders() {
            picker.select(rows.iter().position(|(s, idx, _)| is_current(s, *idx)));
        }
        Self::Picker(Box::new(picker))
    }

    /// The attached clients; Enter detaches one (`choose-client`).
    pub fn client_picker(srv: &Server, choose: &Choose<'_>) -> Self {
        let vars_of = |c: &crate::server::Client| {
            let session = c.att.as_ref().map(|a| a.session);
            Vars::for_pane(
                srv,
                session.and_then(|s| srv.active_pane_of_session(s)),
                Some(c.id),
            )
        };
        let mut clients: Vec<_> = srv
            .clients
            .values()
            .filter(|c| c.att.is_some())
            .filter(|c| choose.passes(&vars_of(c)))
            .collect();
        // tmux's window_client_cmp: by name as the tie breaker.
        let by_name = |a: &&crate::server::Client, b: &&crate::server::Client| {
            a.id.to_string().cmp(&b.id.to_string())
        };
        match choose.sort(CLIENT_SORTS) {
            Some(Sort::Name) => clients.sort_by(by_name),
            Some(Sort::Size) => clients.sort_by(|a, b| {
                let size = |c: &crate::server::Client| c.att.as_ref().map(|a| (a.cols, a.rows));
                size(a).cmp(&size(b)).then_with(|| by_name(a, b))
            }),
            Some(Sort::Creation) => {
                clients.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| by_name(a, b)));
            }
            Some(Sort::Activity) => clients.sort_by(|a, b| {
                let input = |c: &crate::server::Client| c.att.as_ref().map(|a| a.last_input);
                input(b).cmp(&input(a)).then_with(|| by_name(a, b))
            }),
            _ => {}
        }
        if choose.reverse {
            clients.reverse();
        }
        let items = clients
            .iter()
            .filter_map(|c| {
                let att = c.att.as_ref()?;
                let session = srv
                    .sessions
                    .get(&att.session)
                    .map_or("", |s| s.name.as_str());
                let text = choose.text(&vars_of(c), || {
                    format!("{}: {session} [{}x{}]", c.id, att.cols, att.rows)
                });
                Some(Item {
                    label: text,
                    matches: format!("{} {session}", c.id),
                    target: Target::Client(c.id),
                })
            })
            .collect();
        Self::Picker(Box::new(PickerOverlay::new("clients", items)))
    }

    /// The paste buffers, newest first; Enter pastes one into the pane.
    pub fn buffer_picker(srv: &Server, choose: &Choose<'_>) -> Self {
        /// A buffer's variables over the client's.
        fn buffer_vars<'a>(
            b: &'a crate::server::Buffer,
            vars: &'a Vars<'a>,
        ) -> impl Fn(&str) -> Option<String> + 'a {
            move |name| {
                b.var(name)
                    .or_else(|| tmxr_command::format::Context::get(vars, name))
            }
        }
        let vars = Vars::for_pane(srv, None, choose.client);
        let context = |b| buffer_vars(b, &vars);
        // Newest first is tmux's time order.
        let mut buffers: Vec<_> = srv
            .buffers
            .iter()
            .filter(|b| choose.passes(&context(b)))
            .collect();
        let by_name = |a: &&crate::server::Buffer, b: &&crate::server::Buffer| a.name.cmp(&b.name);
        match choose.sort(BUFFER_SORTS) {
            Some(Sort::Name) => buffers.sort_by(by_name),
            Some(Sort::Size) => {
                buffers.sort_by(|a, b| b.data.len().cmp(&a.data.len()).then_with(|| by_name(a, b)));
            }
            _ => {}
        }
        if choose.reverse {
            buffers.reverse();
        }
        let items = buffers
            .iter()
            .map(|b| {
                let preview: String = b
                    .data
                    .chars()
                    .take(50)
                    .map(|c| if c.is_control() { ' ' } else { c })
                    .collect();
                let text = choose.text(&context(b), || {
                    format!("{}: {} bytes: \"{preview}\"", b.name, b.data.len())
                });
                Item {
                    label: text,
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
    /// The options and binds to change, `local` (a target's own options,
    /// as `(line, setting command)`) first.
    pub fn customize_picker(srv: &Server, query: &str, local: Vec<(String, String)>) -> Self {
        let local = local.into_iter().map(|(line, set)| Item {
            label: format!("option {line}"),
            matches: line,
            target: Target::Edit(set),
        });
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
        let items = local.chain(options).chain(binds).collect();
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
            Self::Confirm {
                cmd,
                key,
                default_yes,
                ..
            } => match ev.code {
                KeyCode::Char(c) if c == *key => OverlayAction::Run(cmd.clone()),
                KeyCode::Enter if *default_yes => OverlayAction::Run(cmd.clone()),
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
            Self::Panes { keys: false, .. } => OverlayAction::Pass,
            // A pane's number (0 to 9, then a to z) runs the template for
            // it, as tmux's cmd_display_panes_key; any other key just
            // dismisses.
            Self::Panes {
                labels, template, ..
            } => {
                let pick = match ev.code {
                    KeyCode::Char(c @ '0'..='9') => c.to_digit(10),
                    KeyCode::Char(c @ 'a'..='z') => Some(10 + u32::from(c) - u32::from('a')),
                    _ => None,
                }
                .and_then(|n| {
                    labels
                        .iter()
                        .find(|(label, _)| *label == n)
                        .map(|(_, p)| *p)
                });
                pick.map_or(OverlayAction::Close, |p| {
                    OverlayAction::Run(tmxr_command::fill_template(template, &[format!("%{p}")]))
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

    #[test]
    fn display_panes_takes_letters_past_nine_and_fills_the_template() {
        let labels = vec![(3, 30), (10, 100), (11, 110)];
        let o = || {
            Overlay::display_panes(
                labels.clone(),
                Duration::ZERO,
                "select-pane -t \"%%%\"; set -g @p %1; set -g @q %%".into(),
                true,
            )
        };
        // One answer: the first %% (quoted here) and every %1 take it; a
        // second %% waits for an answer that never comes, as tmux's.
        assert_eq!(
            o().key(&k(KeyCode::Char('b'))),
            OverlayAction::Run("select-pane -t \"%110\"; set -g @p %110; set -g @q %%".into())
        );
        assert_eq!(
            o().key(&k(KeyCode::Char('3'))),
            OverlayAction::Run("select-pane -t \"%30\"; set -g @p %30; set -g @q %%".into())
        );
        assert_eq!(o().key(&k(KeyCode::Char('4'))), OverlayAction::Close);
        assert!(!o().expired(), "-d 0 never times out");
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
    fn single_and_numeric_prompts_answer_as_tmuxs() {
        let prompt = |initial: &str, single: bool, numeric: bool| {
            let mut o = Overlay::prompt("? ".into(), initial.into(), Some("set @k '%%'".into()));
            if let Overlay::Prompt(p) = &mut o {
                p.single = single;
                p.numeric = numeric;
            }
            o
        };
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        // -1: one character is the answer; Ctrl gives its control character.
        assert_eq!(
            prompt("", true, false).key(&k(KeyCode::Char('y'))),
            OverlayAction::Run("set @k 'y'".into())
        );
        assert_eq!(
            prompt("", true, false).key(&ctrl('a')),
            OverlayAction::Run("set @k '\x01'".into())
        );
        assert_eq!(
            prompt("", true, false).key(&k(KeyCode::Left)),
            OverlayAction::Keep
        );
        assert_eq!(
            prompt("x", true, false).key(&k(KeyCode::Char('y'))),
            OverlayAction::Close
        );
        // -N: digits are typed; anything else answers.
        let mut n = prompt("", false, true);
        assert_eq!(n.key(&k(KeyCode::Char('4'))), OverlayAction::Keep);
        assert_eq!(n.key(&k(KeyCode::Char('2'))), OverlayAction::Keep);
        assert_eq!(
            n.key(&k(KeyCode::Char('x'))),
            OverlayAction::Run("set @k '42'".into())
        );
        // With prompts to come, the answer moves on to the next.
        let mut two = prompt("", false, true);
        if let Overlay::Prompt(p) = &mut two {
            p.pending.push_back(("then ".into(), "7".into()));
        }
        assert_eq!(two.key(&k(KeyCode::Char('1'))), OverlayAction::Keep);
        assert_eq!(two.key(&k(KeyCode::Enter)), OverlayAction::Keep);
        assert_eq!(
            two.key(&k(KeyCode::Tab)),
            OverlayAction::Run("set @k '1'".into())
        );
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
    fn confirm_runs_only_on_its_key() {
        let run = || OverlayAction::Run("kill-pane".into());
        let confirm = |key, yes| Overlay::confirm("kill?".into(), "kill-pane".into(), key, yes);
        assert_eq!(confirm('y', false).key(&k(KeyCode::Char('y'))), run());
        assert_eq!(
            confirm('y', false).key(&k(KeyCode::Char('n'))),
            OverlayAction::Close
        );
        // Exactly the key, as tmux's: not its capital.
        assert_eq!(
            confirm('y', false).key(&k(KeyCode::Char('Y'))),
            OverlayAction::Close
        );
        assert_eq!(confirm('x', false).key(&k(KeyCode::Char('x'))), run());
        assert_eq!(
            confirm('x', false).key(&k(KeyCode::Char('y'))),
            OverlayAction::Close
        );
        // Enter confirms only with -y.
        assert_eq!(confirm('y', true).key(&k(KeyCode::Enter)), run());
        assert_eq!(
            confirm('y', false).key(&k(KeyCode::Enter)),
            OverlayAction::Close
        );
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
