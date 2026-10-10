//! `display-menu`: a box of commands, each picked with the arrows and Enter
//! or by its own key, as tmux draws them, placed by `-x` / `-y` as a popup
//! is.

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
    pub look: crate::overlay::BoxLook,
    /// The box's top-left cell; centred when `None`.
    pub at: Option<(u16, u16)>,
}

impl Menu {
    /// Build a menu from `display-menu`'s arguments: `name key command`
    /// triples, where an empty name is a separator taking no key or command.
    /// Names and commands go through `expand`, as tmux's `menu_add_item`:
    /// an item whose name expands to nothing is left out, and a separator is
    /// dropped at the top or after another.
    pub fn parse(
        title: String,
        args: &[String],
        start: usize,
        expand: &dyn Fn(&str) -> String,
    ) -> Result<Self, String> {
        let mut items: Vec<MenuItem> = Vec::new();
        let mut rest = args;
        while let Some((raw, tail)) = rest.split_first() {
            if raw.is_empty() {
                if items.last().is_some_and(|i| *i != MenuItem::Separator) {
                    items.push(MenuItem::Separator);
                }
                rest = tail;
                continue;
            }
            let [key, cmd, tail @ ..] = tail else {
                return Err(format!("display-menu: {raw}: needs a key and a command"));
            };
            rest = tail;
            let name = expand(raw);
            if name.is_empty() {
                continue;
            }
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
                cmd: expand(cmd),
                disabled,
            });
        }
        let mut menu = Self {
            title,
            items,
            selected: 0,
            look: crate::overlay::BoxLook::default(),
            at: None,
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

    fn label(item: &MenuItem) -> (String, String) {
        match item {
            MenuItem::Command { name, key, .. } => match key {
                Some(k) => (name.clone(), format!("({k})")),
                None => (name.clone(), String::new()),
            },
            MenuItem::Separator => (String::new(), String::new()),
        }
    }

    /// The box's width and height, border included.
    pub fn size(&self) -> (u16, u16) {
        let inner_w = self
            .items
            .iter()
            .map(|i| {
                let (name, key) = Self::label(i);
                name.chars().count() + key.chars().count() + 3
            })
            .chain(std::iter::once(
                usize::from(crate::render::runs_width(&self.title)) + 2,
            ))
            .max()
            .unwrap_or(0);
        (
            u16::try_from(inner_w + 2).unwrap_or(u16::MAX),
            u16::try_from(self.items.len() + 2).unwrap_or(u16::MAX),
        )
    }

    /// Draw the menu at [`Menu::at`] (or centred) in the `cols` × `rows`
    /// area: items in `base`, the selected one in `selected`, the border in
    /// `border`.
    pub fn draw(
        &self,
        buf: &mut Buffer,
        cols: u16,
        rows: u16,
        (base, border, selected): (Style, Style, Style),
    ) {
        let (w, h) = self.size();
        let (w, h) = (w.min(cols), h.min(rows));
        let (x, y) = self
            .at
            .unwrap_or_else(|| (cols.saturating_sub(w) / 2, rows.saturating_sub(h) / 2));
        // Kept inside the area should the client have shrunk since.
        let area = Rect::new(
            x.min(cols.saturating_sub(w)),
            y.min(rows.saturating_sub(h)),
            w,
            h,
        );
        Clear.render(area, buf);
        buf.set_style(area, base);
        let block = match self.look.lines {
            Some(lines) => Block::default()
                .borders(Borders::ALL)
                .border_type(lines)
                .border_style(border),
            None => Block::default(),
        };
        let inner = block.inner(area);
        block.render(area, buf);
        // The title on the top border, its `#[…]` styles applied, and
        // centred for tmux's `#[align=centre]` (its default menus use it).
        if self.look.lines.is_some() && !self.title.is_empty() && area.width > 2 {
            let text = format!(" {} ", self.title);
            let width = crate::render::runs_width(&text);
            let centre = self.title.contains("align=centre") || self.title.contains("align=center");
            let right = area.x + area.width - 1;
            let x = if centre {
                area.x + area.width.saturating_sub(width) / 2
            } else {
                area.x + 1
            };
            crate::render::draw_runs(buf, x.max(area.x + 1), area.y, right, base, &text);
        }
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
            let (name, key) = Self::label(item);
            let style = if row == self.selected {
                selected
            } else if !self.selectable(row) {
                base.add_modifier(Modifier::DIM)
            } else {
                base
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
            &|w| w.to_owned(),
        )
        .unwrap();
        assert_eq!(m.items.len(), 4);
        assert_eq!(m.items[1], MenuItem::Separator);
        assert!(!m.selectable(2), "a - name is disabled");
        assert!(Menu::parse("t".into(), &args(&["Only", "o"]), 0, &|w| w.to_owned()).is_err());
        assert!(Menu::parse("t".into(), &args(&["-Off", "", "ls"]), 0, &|w| w.to_owned()).is_err());
    }

    #[test]
    fn items_follow_tmuxs_rules_after_expansion() {
        // "HIDE" expands to nothing, as a false #{?…} would.
        let expand = |w: &str| {
            if w == "HIDE" {
                String::new()
            } else {
                w.replace("CMD", "ran")
            }
        };
        let m = Menu::parse(
            "t".into(),
            &args(&[
                "", "HIDE", "h", "x", "One", "1", "CMD-one", "", "", "HIDE", "z", "y", "Two", "2",
                "two",
            ]),
            0,
            &expand,
        )
        .unwrap();
        // The leading separator goes, the hidden items go, and two
        // separators in a row are one.
        let names: Vec<String> = m
            .items
            .iter()
            .map(|i| match i {
                MenuItem::Command { name, .. } => name.clone(),
                MenuItem::Separator => "--".into(),
            })
            .collect();
        assert_eq!(names, ["One", "--", "Two"]);
        // Commands are expanded too.
        assert_eq!(
            m.items[0],
            MenuItem::Command {
                name: "One".into(),
                key: Some("1".parse().unwrap()),
                cmd: "ran-one".into(),
                disabled: false
            }
        );
    }

    #[test]
    fn keys_move_skip_what_cannot_be_picked_and_run() {
        let mut m = Menu::parse(
            "t".into(),
            &args(&[
                "One", "1", "cmd-one", "", "-Off", "", "ls", "Two", "j", "cmd-two",
            ]),
            0,
            &|w| w.to_owned(),
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
