use super::*;

fn mode(text: &str) -> CopyMode {
    let mut p = vt100::Parser::new(5, 20, 100);
    p.process(text.as_bytes());
    CopyMode::new(p.screen_mut())
}

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

/// Copy mode over `text` with the cursor at the start of its first line.
fn at_start(text: &str) -> CopyMode {
    let mut cm = mode(text);
    (cm.cy, cm.cx) = (0, 0);
    cm
}

#[test]
fn jumps_find_characters_on_the_line_and_repeat() {
    // x at 0, 2, 4, 6.
    let mut cm = at_start("x.x.x.x");
    cm.apply("jump-forward", Some("."));
    assert_eq!(cm.cx, 1);
    cm.apply("jump-again", None);
    assert_eq!(cm.cx, 3);
    cm.apply("jump-reverse", None);
    assert_eq!(cm.cx, 1);

    // t stops before the x; ; then steps past the one beside it.
    let mut cm = at_start("x.x.x.x");
    cm.apply("jump-to-forward", Some("x"));
    assert_eq!(cm.cx, 1);
    cm.apply("jump-again", None);
    assert_eq!(cm.cx, 3);

    // F / T go back.
    cm.cx = 6;
    cm.apply("jump-backward", Some("x"));
    assert_eq!(cm.cx, 4);
    cm.apply("jump-to-backward", Some("x"));
    assert_eq!(cm.cx, 3);
    // No such character: the cursor stays.
    cm.apply("jump-forward", Some("z"));
    assert_eq!(cm.cx, 3);
}

#[test]
fn counts_repeat_commands_and_jumps_wait_for_their_character() {
    let mut cm = at_start("abcdefghijklmnop");
    // 0 alone is start-of-line, for the key table.
    assert!(!cm.take_key(&key('0'), false));
    assert!(cm.take_key(&key('1'), false));
    assert!(cm.take_key(&key('0'), false));
    assert_eq!(cm.count, 10);
    cm.apply_counted("cursor-right", None);
    assert_eq!((cm.cx, cm.count), (10, 0));

    // 2fx: the count waits with the jump for its character.
    let mut cm = at_start("x.x.x.x");
    cm.take_key(&key('2'), false);
    cm.apply_counted("jump-forward", None);
    assert_eq!(cm.pending_jump, Some(Jump::Forward));
    assert!(cm.take_key(&key('x'), false));
    assert_eq!((cm.cx, cm.pending_jump), (4, None));

    // Escape cancels a waiting jump.
    cm.apply_counted("jump-forward", None);
    assert!(cm.take_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), false));
    assert_eq!((cm.cx, cm.pending_jump), (4, None));
}

#[test]
fn emacs_counts_are_typed_with_alt() {
    let alt = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT);
    let mut cm = at_start("abcdefghijklmnop");
    // Plain digits are not counts in emacs mode.
    assert!(!cm.take_key(&key('3'), true));
    assert!(cm.take_key(&alt('1'), true));
    assert!(cm.take_key(&alt('2'), true));
    assert_eq!(cm.count, 12);
    // And Alt digits are not counts in vi mode.
    assert!(!cm.take_key(&alt('4'), false));
    cm.apply_counted("cursor-right", None);
    assert_eq!(cm.cx, 12);
}

#[test]
fn percent_finds_the_matching_bracket_across_lines() {
    let mut cm = at_start("a (b [c] d) e");
    cm.apply("next-matching-bracket", None);
    assert_eq!(cm.cx, 10, "from before the ( to its )");
    cm.apply("next-matching-bracket", None);
    assert_eq!(cm.cx, 2, "and back");
    cm.cx = 5;
    cm.apply("next-matching-bracket", None);
    assert_eq!(cm.cx, 7, "inner pair");

    // Same-type nesting: the outer ( matches the outer ).
    let mut cm = at_start("((x))");
    cm.apply("next-matching-bracket", None);
    assert_eq!(cm.cx, 4);

    let mut cm = at_start("{\r\n  x\r\n}");
    cm.apply("next-matching-bracket", None);
    assert_eq!((cm.cy, cm.cx), (2, 0));
}

#[test]
fn jump_to_mark_swaps_with_the_cursor() {
    let mut cm = at_start("abcdefgh");
    cm.cx = 2;
    cm.apply("set-mark", None);
    cm.cx = 6;
    cm.apply("jump-to-mark", None);
    assert_eq!(cm.cx, 2);
    cm.apply("jump-to-mark", None);
    assert_eq!(cm.cx, 6);
}

#[test]
fn snapshot_includes_history_and_screen() {
    let lines: String = (0..12).map(|i| format!("line{i}\r\n")).collect();
    let cm = mode(&lines);
    let texts: Vec<String> = cm.lines.iter().map(|l| l.text(0, 20)).collect();
    assert_eq!(texts[0], "line0");
    assert_eq!(texts[11], "line11");
    assert_eq!(cm.position(), (0, cm.lines.len() - 5));
}

#[test]
fn word_motions() {
    let mut cm = mode("foo bar.baz  qux");
    (cm.cy, cm.cx) = (0, 0);
    cm.apply("next-word", None);
    assert_eq!(cm.cx, 4);
    cm.apply("next-word", None);
    assert_eq!(cm.cx, 7, "punctuation is its own word");
    cm.apply("next-space", None);
    assert_eq!(cm.cx, 13, "WORD skips punctuation");
    cm.apply("previous-word", None);
    assert_eq!(cm.cx, 8);
    cm.apply("next-word-end", None);
    assert_eq!(cm.cx, 10);
    cm.apply("start-of-line", None);
    cm.apply("end-of-line", None);
    assert_eq!(cm.cx, 15);
}

#[test]
fn selections_copy_char_line_and_rectangle_text() {
    let mut cm = mode("abcdef\r\nghijkl\r\nmnopqr");
    (cm.cy, cm.cx) = (0, 2);
    cm.apply("begin-selection", None);
    (cm.cy, cm.cx) = (1, 1);
    assert_eq!(cm.selection_text().unwrap(), "cdef\ngh");
    cm.apply("rectangle-toggle", None);
    assert_eq!(cm.selection_text().unwrap(), "bc\nhi");
    // select-line anchors on the cursor's line (1, after the move above).
    cm.apply("select-line", None);
    (cm.cy, cm.cx) = (2, 0);
    assert_eq!(cm.selection_text().unwrap(), "ghijkl\nmnopqr\n");
}

#[test]
fn search_finds_forward_backward_and_wraps() {
    let mut cm = mode("one\r\ntwo\r\none two");
    (cm.cy, cm.cx) = (0, 0);
    cm.apply("search-forward", Some("two"));
    assert_eq!((cm.cy, cm.cx), (1, 0));
    cm.apply("search-again", None);
    assert_eq!((cm.cy, cm.cx), (2, 4));
    cm.apply("search-again", None);
    assert_eq!((cm.cy, cm.cx), (1, 0), "wraps around");
    cm.apply("search-reverse", None);
    assert_eq!((cm.cy, cm.cx), (2, 4));
}
