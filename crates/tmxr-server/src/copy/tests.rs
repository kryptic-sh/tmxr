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
fn previous_matching_bracket_looks_back_from_the_cursor() {
    let mut cm = at_start("a (b [c] d) e");
    cm.cx = 12;
    cm.apply("previous-matching-bracket", None);
    assert_eq!(cm.cx, 2, "from after the ) back to its (");
    cm.cx = 7;
    cm.apply("previous-matching-bracket", None);
    assert_eq!(cm.cx, 5, "on a ] itself");
}

#[test]
fn select_word_takes_the_word_under_the_cursor() {
    let mut cm = at_start("foo bar_baz.qux");
    cm.cx = 6;
    cm.apply("select-word", None);
    assert_eq!(cm.selection_text().unwrap(), "bar_baz");
    cm.cx = 12;
    cm.apply("select-word", None);
    assert_eq!(
        cm.selection_text().unwrap(),
        "qux",
        "at the end of the line"
    );
}

#[test]
fn paragraphs_move_between_blank_lines() {
    let mut cm = at_start("one\r\ntwo\r\n\r\nthree\r\n\r\n\r\nfour");
    cm.apply("next-paragraph", None);
    assert_eq!(cm.cy, 2);
    cm.apply("next-paragraph", None);
    assert_eq!(cm.cy, 4, "past the blank line, to the next one");
    cm.apply("previous-paragraph", None);
    assert_eq!(cm.cy, 2);
    cm.apply("previous-paragraph", None);
    assert_eq!(cm.cy, 0, "the first line when no blank is left");
}

#[test]
fn goto_line_scrolls_that_far_above_the_bottom() {
    let lines: String = (0..12).map(|i| format!("line{i}\r\n")).collect();
    let mut cm = mode(&lines);
    let (_, bottom_top) = cm.position();
    assert!(cm.apply("goto-line", Some("3")));
    assert_eq!(cm.position(), (3, bottom_top));
    assert!(cm.apply("goto-line", Some("999")));
    assert_eq!(cm.top, 0, "clamped to the top of the history");
    assert!(!cm.apply("goto-line", Some("x")));
}

#[test]
fn incremental_search_stays_while_the_text_still_matches() {
    let mut cm = at_start("ab abc abcd");
    cm.apply("search-forward-incremental", Some("a"));
    assert_eq!(cm.cx, 0, "a match at the cursor counts");
    cm.apply("search-forward-incremental", Some("ab"));
    assert_eq!(cm.cx, 0);
    cm.apply("search-forward-incremental", Some("abc"));
    assert_eq!(cm.cx, 3, "moves on once the text stops matching");
    cm.apply("search-forward-incremental", Some("abcd"));
    assert_eq!(cm.cx, 7);
    // n repeats it as an ordinary search.
    cm.apply("search-again", None);
    assert_eq!(cm.cx, 7, "the only abcd, found again by wrapping");
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

#[test]
fn other_end_swaps_cursor_and_anchor_and_a_count_by_parity() {
    let mut cm = at_start("hello world");
    cm.apply("begin-selection", None);
    cm.cx = 4;
    cm.apply("other-end", None);
    assert_eq!((cm.cx, cm.anchor), (0, Some((0, 4))));
    // Twice is back where it was, as tmux's prefix parity.
    cm.count = 2;
    cm.apply_counted("other-end", None);
    assert_eq!((cm.cx, cm.anchor), (0, Some((0, 4))));
    // Nothing to swap without a selection.
    cm.apply("clear-selection", None);
    cm.apply("other-end", None);
    assert_eq!((cm.cx, cm.anchor), (0, None));
}

#[test]
fn centring_and_scroll_middle_place_the_cursor_and_the_view() {
    // 30 numbered lines in a 5-row, 20-column pane.
    let text: String = (0..30).map(|i| format!("{i}\r\n")).collect();
    let mut cm = mode(&text);
    cm.apply("history-top", None);
    cm.apply("cursor-centre-vertical", None);
    assert_eq!(cm.cy, 2, "the middle row of the view");
    cm.apply("cursor-centre-horizontal", None);
    // tmux's: half the pane's width, whatever the line holds.
    assert_eq!(cm.cx, 10, "the middle column");
    // The cursor's line comes to the middle of the view.
    cm.cy = 15;
    cm.apply("scroll-middle", None);
    assert_eq!((cm.top, cm.cy), (13, 15));
}

#[test]
fn toggle_position_hides_and_shows_the_indicator() {
    let mut cm = at_start("x");
    assert!(!cm.hide_position);
    cm.apply("toggle-position", None);
    assert!(cm.hide_position);
    cm.apply("toggle-position", None);
    assert!(!cm.hide_position);
}

#[test]
fn the_cursor_word_is_tmuxs() {
    let word_at = |text: &str, x: usize| {
        let mut cm = at_start(text);
        cm.cx = x;
        cm.cursor_word()
    };
    // Punctuation (but `_`) and blanks separate words; other characters,
    // non-ASCII ones too, do not.
    assert_eq!(word_at("foo.bar baz", 1), "foo");
    assert_eq!(word_at("foo.bar baz", 9), "baz");
    assert_eq!(word_at("snake_case x", 3), "snake_case");
    assert_eq!(word_at("héllo→wörld", 2), "héllo→wörld");
    // On a separator: the word just after it.
    assert_eq!(word_at("foo.bar baz", 3), "bar");
    assert_eq!(word_at("foo  bar", 3), "");
}
