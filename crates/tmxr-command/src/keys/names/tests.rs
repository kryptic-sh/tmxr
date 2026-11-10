use super::*;

fn k(s: &str) -> Key {
    s.parse().unwrap_or_else(|e| panic!("{s}: {e}"))
}

/// Every key printed parses back to itself.
fn round_trips(key: Key) {
    let name = key.to_string();
    assert_eq!(k(&name), key, "{name}");
}

#[test]
fn tmux_names_tmux_3_6_keeps_apart_stay_apart() {
    // tmux 3.6's list-keys printed each of these back as bound, as its
    // own key (scratchpad build, 2026-11-10); the ones a terminal can send
    // are also the ones tmux reads from it.
    for name in [
        "C-h", "C-S-h", "C-H", "C-a", "C-A", "C-S-a", "C-4", "C-\\", "C-@", "C-Space", "C-2",
        "C-1", "C-5", "C-6", "C-7", "C-8", "C-[", "C-?", "C-!", "C-i", "C-m", "C-Tab", "C-Enter",
        "C-Escape", "C-BSpace", "S-Enter", "S-F1", "C-S-Up", "C-M-S-x", "M-H", "M-Space", "KP1",
        "KPEnter", "KP/", "KP*", "KP-", "KP+", "KP.", "KP0", "C-é", "S-Tab", "C-S-Tab", "BTab",
        "C-S-1", "C-M-S-1",
    ] {
        assert_eq!(k(name).to_string(), name, "{name}");
    }
    // Ctrl with Shift and a letter is one key however the letter's case
    // is written: tmux keeps the names apart, tmxr reads each as the key a
    // terminal sends for it. Without Ctrl, tmux's Shift beside a character
    // is dropped, as tmux's send-keys sends M-S-a as M-a.
    for (alias, name) in [
        ("C-S-H", "C-S-h"),
        ("M-S-h", "M-h"),
        ("M-S-H", "M-H"),
        ("S-h", "h"),
        ("^A", "C-a"),
        ("^4", "C-4"),
    ] {
        assert_eq!(k(alias), k(name), "{alias}");
        assert_eq!(k(alias).to_string(), name, "{alias}");
    }
    assert_ne!(k("S-Tab"), k("BTab"));
    assert_ne!(k("C-S-1"), k("C-1"));
}

#[test]
fn send_keys_takes_only_the_names_tmux_knows() {
    // tmux 3.6's send-keys sent each of these as a key.
    for name in [
        "Enter", "enter", "BSpace", "PgUp", "IC", "F1", "f12", "C-a", "M-Up", "C-S-h", "^A",
        "KPEnter", "KP1", "kp/", "S-Tab", "C-M-S-x", "Space",
    ] {
        assert_eq!(Key::from_tmux_name(name), Some(k(name)), "{name}");
    }
    // And typed each of these as text (scratchpad build, 2026-11-10):
    // kitty's, hjkl's and tmxr's own names among them.
    for word in [
        "menu",
        "pause",
        "Pause",
        "capslock",
        "CapsLock",
        "f13",
        "F13",
        "F35",
        "media_play",
        "MediaPlay",
        "plus",
        "ctrl+c",
        "shift+tab",
        "backspace",
        "kp_enter",
        "KPLeft",
        "KP=",
        "Super-k",
        "Meta-x",
        "<C-b>",
        "<CR>",
    ] {
        assert_eq!(Key::from_tmux_name(word), None, "{word}");
    }
}

#[test]
fn extended_tmux_names_cover_every_new_key_class() {
    for name in [
        // Super, Hyper and Meta, alone, together, and after tmux's.
        "Super-k",
        "Hyper-k",
        "Meta-k",
        "Super-Hyper-Meta-k",
        "C-S-Super-h",
        "M-Super-Up",
        "Super-A",
        // F13 to F35.
        "F13",
        "F24",
        "F35",
        "C-F20",
        // The keypad keys tmux has no name for.
        "KP=",
        "KP,",
        "KPLeft",
        "KPRight",
        "KPUp",
        "KPDown",
        "KPPPage",
        "KPNPage",
        "KPHome",
        "KPEnd",
        "KPIC",
        "KPDC",
        "KPBegin",
        "C-KP5",
        // The lock and other keys.
        "CapsLock",
        "ScrollLock",
        "NumLock",
        "PrintScreen",
        "Pause",
        "Menu",
        // Media keys.
        "MediaPlay",
        "MediaPause",
        "MediaPlayPause",
        "MediaReverse",
        "MediaStop",
        "MediaFastForward",
        "MediaRewind",
        "MediaTrackNext",
        "MediaTrackPrevious",
        "MediaRecord",
        "LowerVolume",
        "RaiseVolume",
        "MuteVolume",
        // Modifier keys.
        "LeftShift",
        "LeftControl",
        "LeftAlt",
        "LeftSuper",
        "LeftHyper",
        "LeftMeta",
        "RightShift",
        "RightControl",
        "RightAlt",
        "RightSuper",
        "RightHyper",
        "RightMeta",
        "IsoLevel3Shift",
        "IsoLevel5Shift",
    ] {
        assert_eq!(k(name).to_string(), name, "{name}");
    }
    // In any case, as tmux's names; and a keypad key by any main name.
    assert_eq!(k("super-k"), k("Super-k"));
    assert_eq!(k("META-k"), k("Meta-k"));
    assert_eq!(k("kpenter"), k("KPEnter"));
    assert_eq!(k("KPPageUp"), k("KPPPage"));
    assert_eq!(k("f13"), k("F13"));
    // Meta- is kitty's Meta, apart from tmux's M- (Alt).
    assert_ne!(k("Meta-k"), k("M-k"));
    for bad in ["F0", "F36", "KPx", "KPF1", "KPTab", "Super-", "X-a", "C-"] {
        assert!(bad.parse::<Key>().is_err(), "{bad}");
    }
}

#[test]
fn every_kitty_name_parses_to_its_key() {
    // kitty's functional key table, as the spec lists it, with the tmux
    // or extended name tmxr prints for each.
    let mut table: Vec<(String, &str)> = [
        ("escape", "Escape"),
        ("enter", "Enter"),
        ("tab", "Tab"),
        ("backspace", "BSpace"),
        ("insert", "IC"),
        ("delete", "DC"),
        ("left", "Left"),
        ("right", "Right"),
        ("up", "Up"),
        ("down", "Down"),
        ("page_up", "PPage"),
        ("page_down", "NPage"),
        ("home", "Home"),
        ("end", "End"),
        ("caps_lock", "CapsLock"),
        ("scroll_lock", "ScrollLock"),
        ("num_lock", "NumLock"),
        ("print_screen", "PrintScreen"),
        ("pause", "Pause"),
        ("menu", "Menu"),
        ("kp_decimal", "KP."),
        ("kp_divide", "KP/"),
        ("kp_multiply", "KP*"),
        ("kp_subtract", "KP-"),
        ("kp_add", "KP+"),
        ("kp_enter", "KPEnter"),
        ("kp_equal", "KP="),
        ("kp_separator", "KP,"),
        ("kp_left", "KPLeft"),
        ("kp_right", "KPRight"),
        ("kp_up", "KPUp"),
        ("kp_down", "KPDown"),
        ("kp_page_up", "KPPPage"),
        ("kp_page_down", "KPNPage"),
        ("kp_home", "KPHome"),
        ("kp_end", "KPEnd"),
        ("kp_insert", "KPIC"),
        ("kp_delete", "KPDC"),
        ("kp_begin", "KPBegin"),
        ("media_play", "MediaPlay"),
        ("media_pause", "MediaPause"),
        ("media_play_pause", "MediaPlayPause"),
        ("media_reverse", "MediaReverse"),
        ("media_stop", "MediaStop"),
        ("media_fast_forward", "MediaFastForward"),
        ("media_rewind", "MediaRewind"),
        ("media_track_next", "MediaTrackNext"),
        ("media_track_previous", "MediaTrackPrevious"),
        ("media_record", "MediaRecord"),
        ("lower_volume", "LowerVolume"),
        ("raise_volume", "RaiseVolume"),
        ("mute_volume", "MuteVolume"),
        ("left_shift", "LeftShift"),
        ("left_control", "LeftControl"),
        ("left_alt", "LeftAlt"),
        ("left_super", "LeftSuper"),
        ("left_hyper", "LeftHyper"),
        ("left_meta", "LeftMeta"),
        ("right_shift", "RightShift"),
        ("right_control", "RightControl"),
        ("right_alt", "RightAlt"),
        ("right_super", "RightSuper"),
        ("right_hyper", "RightHyper"),
        ("right_meta", "RightMeta"),
        ("iso_level3_shift", "IsoLevel3Shift"),
        ("iso_level5_shift", "IsoLevel5Shift"),
        ("space", "Space"),
        ("plus", "+"),
    ]
    .into_iter()
    .map(|(kitty, tmux)| (kitty.to_owned(), tmux))
    .collect();
    let f: Vec<String> = (1..=35).map(|n| format!("F{n}")).collect();
    let kp: Vec<String> = (0..=9).map(|n| format!("KP{n}")).collect();
    for (n, name) in f.iter().enumerate() {
        table.push((format!("f{}", n + 1), name));
    }
    for (n, name) in kp.iter().enumerate() {
        table.push((format!("kp_{n}"), name));
    }
    // Every name in the table is tested, each a key of its own.
    assert_eq!(table.len(), KITTY.len() + 35 + 10);
    let mut seen = std::collections::HashSet::new();
    for (kitty, tmux) in &table {
        let key = k(kitty);
        assert_eq!(key.to_string(), *tmux, "{kitty}");
        assert_eq!(k(&kitty.to_ascii_uppercase()), key, "{kitty}");
        assert_eq!(
            k(&format!("ctrl+{kitty}")),
            k(&format!("C-{tmux}")),
            "{kitty}"
        );
        assert!(seen.insert(key), "{kitty} names another's key");
        round_trips(key);
    }
}

#[test]
fn kitty_modifiers_name_the_same_keys_as_tmuxs() {
    for (kitty, tmux) in [
        ("ctrl+a", "C-a"),
        ("ctrl+shift+h", "C-S-h"),
        ("shift+ctrl+h", "C-S-h"),
        ("CTRL+SHIFT+H", "C-S-h"),
        ("alt+shift+h", "M-H"),
        ("opt+a", "M-a"),
        ("option+a", "M-a"),
        ("alt+a", "M-a"),
        ("shift+a", "A"),
        ("super+k", "Super-k"),
        ("cmd+k", "Super-k"),
        ("command+k", "Super-k"),
        ("hyper+k", "Hyper-k"),
        ("meta+k", "Meta-k"),
        (
            "ctrl+alt+super+hyper+meta+shift+x",
            "C-M-S-Super-Hyper-Meta-x",
        ),
        ("ctrl+plus", "C-+"),
        ("ctrl+space", "C-Space"),
        ("shift+tab", "BTab"),
        ("ctrl+f13", "C-F13"),
        ("super+kp_enter", "Super-KPEnter"),
        ("ctrl+4", "C-4"),
    ] {
        let tmux_key = k(tmux);
        assert_eq!(k(kitty), tmux_key, "{kitty}");
        assert_eq!(tmux_key.to_string(), tmux, "{kitty}");
    }
    // One key, two spellings: either replaces a bind on the other.
    assert_eq!(k("M-H"), k("alt+shift+h"));
    // tmux's names with a `+` stay tmux's.
    for name in ["+", "M-+", "C-+", "KP+", "C-M-+"] {
        assert_eq!(k(name).code, KeyCode::Char('+'), "{name}");
    }
    // The lock modifiers are no part of a key, so kitty's names for them
    // name no modifier.
    for bad in [
        "caps_lock+a",
        "num_lock+a",
        "ctrl+",
        "+a",
        "a+b",
        "ctrl+nope",
    ] {
        assert!(bad.parse::<Key>().is_err(), "{bad}");
    }
}

#[test]
fn hjkl_names_follow_neovim() {
    for (hjkl, tmux) in [
        ("<C-b>", "C-b"),
        ("<c-b>", "C-b"),
        ("<M-h>", "M-h"),
        ("<A-h>", "M-h"),
        ("<CR>", "Enter"),
        ("<Esc>", "Escape"),
        ("<BS>", "BSpace"),
        ("<Space>", "Space"),
        ("<C-Space>", "C-Space"),
        ("<lt>", "<"),
        ("<Bar>", "|"),
        ("<Bslash>", "\\"),
        ("<F5>", "F5"),
        ("<C-M-x>", "C-M-x"),
        ("<M-Up>", "M-Up"),
        // A shifted letter is its upper case, however written; Ctrl with
        // Shift and a letter is apart from Ctrl with the letter.
        ("<M-A>", "M-A"),
        ("<M-S-a>", "M-A"),
        ("<S-M-a>", "M-A"),
        ("<C-S-h>", "C-S-h"),
        ("<S-C-h>", "C-S-h"),
        // neovim's D- (Super) and T- (Meta), in any case and order.
        ("<D-a>", "Super-a"),
        ("<d-a>", "Super-a"),
        ("<T-a>", "Meta-a"),
        ("<D-T-C-x>", "C-Super-Meta-x"),
        // neovim's names for F13 and up and the keypad.
        ("<F13>", "F13"),
        ("<F35>", "F35"),
        ("<kEnter>", "KPEnter"),
        ("<k0>", "KP0"),
        ("<k9>", "KP9"),
        ("<kPlus>", "KP+"),
        ("<kMinus>", "KP-"),
        ("<kMultiply>", "KP*"),
        ("<kDivide>", "KP/"),
        ("<kPoint>", "KP."),
        ("<kComma>", "KP,"),
        ("<kEqual>", "KP="),
        ("<kUp>", "KPUp"),
        ("<kDown>", "KPDown"),
        ("<kLeft>", "KPLeft"),
        ("<kRight>", "KPRight"),
        ("<kHome>", "KPHome"),
        ("<kEnd>", "KPEnd"),
        ("<kPageUp>", "KPPPage"),
        ("<kPageDown>", "KPNPage"),
        ("<kInsert>", "KPIC"),
        ("<kDel>", "KPDC"),
        ("<kOrigin>", "KPBegin"),
        // Keys neovim has no name for, by tmxr's.
        ("<MediaPlay>", "MediaPlay"),
        ("<KPEnter>", "KPEnter"),
        ("<C-KP1>", "C-KP1"),
        ("<KPLeft>", "KPLeft"),
        ("<S-Tab>", "BTab"),
        ("<D-CapsLock>", "Super-CapsLock"),
    ] {
        assert_eq!(k(hjkl), k(tmux), "{hjkl}");
    }
    assert_ne!(k("<C-S-h>"), k("<C-h>"));
    // Not kitty's names inside `<…>`, as hjkl's.
    for bad in ["<Nope>", "<alt+shift+h>", "<F36>", "<k10>"] {
        assert!(bad.parse::<Key>().is_err(), "{bad}");
    }
    // A lone < or > is still the key itself.
    assert_eq!(k("<").code, KeyCode::Char('<'));
    assert_eq!(k(">").code, KeyCode::Char('>'));
    assert!("".parse::<Key>().is_err());
}
