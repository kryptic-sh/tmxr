//! How a menu or popup box is drawn: its look from the command's flags and
//! tmux's box line sets.

use ratatui::symbols::border;
use tmxr_config::BoxLines;

/// How a menu or popup box is drawn, from tmux's `-s` (style), `-S` (border
/// style), `-H` (a menu's selected item) and `-b` (border lines) flags: style
/// specs as given, expanded when drawn; `None` takes the `menu-*` or
/// `popup-*` option's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoxLook {
    pub style: Option<String>,
    pub border_style: Option<String>,
    pub selected_style: Option<String>,
    /// The border's lines; `None` is `-b none`.
    pub lines: Option<Lines>,
}

impl BoxLook {
    /// From a command's flags; without `-b`, the lines `option` holds
    /// (`menu-border-lines`, …), whose name an unknown `-b` errors with, as
    /// tmux's.
    pub fn from_args(
        a: &tmxr_command::Args,
        option: &str,
        default: BoxLines,
    ) -> Result<Self, String> {
        let lines = match a.value('b') {
            Some(name) => {
                BoxLines::parse(name).ok_or_else(|| format!("{option} unknown value: {name}"))?
            }
            None => default,
        };
        let spec = |f| a.value(f).map(str::to_owned);
        Ok(Self {
            style: spec('s'),
            border_style: spec('S'),
            selected_style: spec('H'),
            lines: Lines::of(lines),
        })
    }
}

/// tmux's box lines: the characters a menu or popup border is drawn with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Lines {
    #[default]
    Single,
    Rounded,
    Double,
    Heavy,
    /// ASCII: `+`, `|` and `-`.
    Simple,
    /// Blank cells, the box keeping its size.
    Padded,
}

impl Lines {
    /// The lines an option or `-b` names; `None` for `none`.
    pub const fn of(lines: BoxLines) -> Option<Self> {
        Some(match lines {
            BoxLines::Single => Self::Single,
            BoxLines::Rounded => Self::Rounded,
            BoxLines::Double => Self::Double,
            BoxLines::Heavy => Self::Heavy,
            BoxLines::Simple => Self::Simple,
            BoxLines::Padded => Self::Padded,
            BoxLines::None => return None,
        })
    }

    /// The border, as tmux 3.6 draws it.
    pub fn border(self) -> border::Set<'static> {
        match self {
            Self::Single => border::PLAIN,
            Self::Rounded => border::ROUNDED,
            Self::Double => border::DOUBLE,
            Self::Heavy => border::THICK,
            Self::Simple => border::Set {
                top_left: "+",
                top_right: "+",
                bottom_left: "+",
                bottom_right: "+",
                vertical_left: "|",
                vertical_right: "|",
                horizontal_top: "-",
                horizontal_bottom: "-",
            },
            Self::Padded => border::EMPTY,
        }
    }

    /// A menu separator across the box: its left end, fill and right end,
    /// as tmux 3.6 draws them.
    pub fn separator(self) -> [&'static str; 3] {
        match self {
            Self::Single | Self::Rounded => ["├", "─", "┤"],
            Self::Double => ["╠", "═", "╣"],
            Self::Heavy => ["┣", "━", "┫"],
            Self::Simple => ["+", "-", "+"],
            Self::Padded => [" ", " ", " "],
        }
    }
}
