//! `display-menu`: a box of commands, each picked with the arrows and Enter
//! or by its own key, as tmux draws them (centred here; tmux's `-x` / `-y`
//! placement and styles are not followed).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Widget};
use tmxr_command::Key;

use crate::overlay::OverlayAction;

/// One menu line: a command with a name and maybe a key, or a separator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuItem {
    Command {
        name: String,
        key: Option<Key>,
        cmd: String,
        /// A name starting with `-` (tmux): shown dimmed, never picked.
        disabled: bool,
    },
    Separator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    pub title: String,
    pub items: Vec<MenuItem>,
    pub selected: usize,
}

impl Menu {
    /// Build a menu from `display-menu`'s arguments: `name key command`
    /// triples, where an empty name is a separator taking no key or command.
    pub fn parse(title: String, args: &[String], start: usize) -> Result<Self, String> {
        let mut items = Vec::new();
        let mut rest = args;
        while let Some((name, tail)) = rest.split_first() {
            if name.is_empty() {
                items.push(MenuItem::Separator);
                rest = tail;
                continue;
            }
            let [key, cmd, tail @ ..] = tail else {
                return Err(format!("display-menu: {name}: needs a key and a command"));
            };
            let key = if key.is_empty() {
                None
            } else {
                Some(
                    key.parse::<Key>()
                        .map_err(|e| format!("display-menu: {name}: {e}"))?,
                )
            };
            let (disabled, name) = match name.strip_prefix('-') {
                Some(n) => (true, n.to_owned()),
                None => (false, name.clone()),
            };
            items.push(MenuItem::Command {
                name,
                key,
                cmd: cmd.clone(),
                disabled,
            });
            rest = tail;
        }
        let mut menu = Self {
            title,
            items,
            selected: 0,
        };
        if !(0..menu.items.len()).any(|i| menu.selectable(i)) {
            return Err("display-menu: no items to choose".into());
        }
        menu.selected = start.min(menu.items.len() - 1);
        if !menu.selectable(menu.selected) {
            menu.step(true);
        }
        Ok(menu)
    }

    fn selectable(&self, i: usize) -> bool {
        matches!(
            self.items.get(i),
            Some(MenuItem::Command {
                disabled: false,
                ..
            })
        )
    }

    /// Move to the next (`down`) or previous selectable item, wrapping.
    fn step(&mut self, down: bool) {
        let n = self.items.len();
        let mut i = self.selected;
        for _ in 0..n {
            i = if down { (i + 1) % n } else { (i + n - 1) % n };
            if self.selectable(i) {
                self.selected = i;
                return;
            }
        }
    }

    fn run(&self, i: usize) -> OverlayAction {
        match &self.items[i] {
            MenuItem::Command { cmd, .. } => OverlayAction::Run(cmd.clone()),
            MenuItem::Separator => OverlayAction::Keep,
        }
    }

    pub fn key(&mut self, ev: &KeyEvent) -> OverlayAction {
        // An item's own key comes first, so an item may use j, k or q.
        let pressed = Key::from_event(ev);
        if let Some(i) = (0..self.items.len()).find(|&i| {
            self.selectable(i)
                && matches!(&self.items[i], MenuItem::Command { key: Some(k), .. } if *k == pressed)
        }) {
            return self.run(i);
        }
        let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
        match ev.code {
            KeyCode::Esc | KeyCode::Char('q') => return OverlayAction::Close,
            KeyCode::Char('c') if ctrl => return OverlayAction::Close,
            KeyCode::Enter => return self.run(self.selected),
            KeyCode::Up | KeyCode::Char('k') => self.step(false),
            KeyCode::Char('p') if ctrl => self.step(false),
            KeyCode::Down | KeyCode::Char('j') => self.step(true),
            KeyCode::Char('n') if ctrl => self.step(true),
            _ => {}
        }
        OverlayAction::Keep
    }

    /// Draw the menu centred in the `cols` × `rows` pane area.
    pub fn draw(&self, buf: &mut Buffer, cols: u16, rows: u16, border: Style, selected: Style) {
        let label = |item: &MenuItem| match item {
            MenuItem::Command { name, key, .. } => match key {
                Some(k) => (name.clone(), format!("({k})")),
                None => (name.clone(), String::new()),
            },
            MenuItem::Separator => (String::new(), String::new()),
        };
        let inner_w = self
            .items
            .iter()
            .map(|i| {
                let (name, key) = label(i);
                name.chars().count() + key.chars().count() + 3
            })
            .chain(std::iter::once(self.title.chars().count() + 2))
            .max()
            .unwrap_or(0);
        let w = u16::try_from(inner_w + 2).unwrap_or(u16::MAX).min(cols);
        let h = u16::try_from(self.items.len() + 2)
            .unwrap_or(u16::MAX)
            .min(rows);
        let area = Rect::new(cols.saturating_sub(w) / 2, rows.saturating_sub(h) / 2, w, h);
        Clear.render(area, buf);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .title(format!(" {} ", self.title));
        let inner = block.inner(area);
        block.render(area, buf);
        for (row, item) in self
            .items
            .iter()
            .enumerate()
            .take(usize::from(inner.height))
        {
            let y = inner.y + u16::try_from(row).unwrap_or(u16::MAX);
            if let MenuItem::Separator = item {
                let line = "─".repeat(usize::from(inner.width));
                buf.set_stringn(inner.x, y, &line, usize::from(inner.width), border);
                continue;
            }
            let (name, key) = label(item);
            let style = if row == self.selected {
                selected
            } else if !self.selectable(row) {
                Style::default().add_modifier(Modifier::DIM)
            } else {
                Style::default()
            };
            buf.set_style(Rect::new(inner.x, y, inner.width, 1), style);
            buf.set_stringn(inner.x + 1, y, &name, usize::from(inner.width), style);
            let kw = u16::try_from(key.chars().count()).unwrap_or(0);
            if kw + 2 < inner.width {
                buf.set_stringn(
                    inner.x + inner.width - kw - 1,
                    y,
                    &key,
                    usize::from(kw),
                    style,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn items_parse_with_separators_and_disabled_entries() {
        let m = Menu::parse(
            "t".into(),
            &args(&[
                "Split",
                "s",
                "split-window",
                "",
                "-Off",
                "",
                "ls",
                "Kill",
                "x",
                "kill-pane",
            ]),
            0,
        )
        .unwrap();
        assert_eq!(m.items.len(), 4);
        assert_eq!(m.items[1], MenuItem::Separator);
        assert!(!m.selectable(2), "a - name is disabled");
        assert!(Menu::parse("t".into(), &args(&["Only", "o"]), 0).is_err());
        assert!(Menu::parse("t".into(), &args(&["-Off", "", "ls"]), 0).is_err());
    }

    #[test]
    fn keys_move_skip_what_cannot_be_picked_and_run() {
        let mut m = Menu::parse(
            "t".into(),
            &args(&[
                "One", "1", "cmd-one", "", "-Off", "", "ls", "Two", "j", "cmd-two",
            ]),
            0,
        )
        .unwrap();
        assert_eq!(m.key(&k(KeyCode::Down)), OverlayAction::Keep);
        assert_eq!(m.selected, 3, "past the separator and the disabled item");
        assert_eq!(
            m.key(&k(KeyCode::Enter)),
            OverlayAction::Run("cmd-two".into())
        );
        // An item's key wins over j-for-down.
        assert_eq!(
            m.key(&k(KeyCode::Char('j'))),
            OverlayAction::Run("cmd-two".into())
        );
        assert_eq!(
            m.key(&k(KeyCode::Char('1'))),
            OverlayAction::Run("cmd-one".into())
        );
        assert_eq!(m.key(&k(KeyCode::Esc)), OverlayAction::Close);
    }
}
