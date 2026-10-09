use super::*;
#[path = "before.rs"]
mod before;
#[path = "fixtures.rs"]
mod fixtures;

#[test]
fn full_enrichment_matches_frozen_predecessor_for_1536_source_and_unicode_inputs() {
    for seed in 0..24 {
        for node_count in [0, 1, 8, 32] {
            for line_count in [0, 1, 12, 65] {
                for mode in 0..4 {
                    let mut input = fixtures::fixture(node_count, line_count, seed);
                    match mode {
                        1 => {
                            for node in &mut input.board.nodes {
                                node.source_excerpt = " supplied 全文 👩🏽‍💻 ".repeat(12);
                            }
                        }
                        2 => {
                            for node in &mut input.board.nodes {
                                node.source_excerpt = " \t　 ".into();
                                node.source_type = "lecture".into();
                            }
                        }
                        3 => input.terms.clear(),
                        _ => {}
                    }
                    let previous = (mode != 3).then_some(&input.previous);
                    let original =
                        serde_json::to_vec(&(&input.previous, &input.lines, &input.terms)).unwrap();
                    let old = before::enrich_whiteboard_source_excerpts(
                        Some(input.board.clone()),
                        previous,
                        &input.terms,
                        &input.lines,
                    );
                    let new = enrich_whiteboard_source_excerpts(
                        Some(input.board),
                        previous,
                        &input.terms,
                        &input.lines,
                    );
                    assert_eq!(
                        serde_json::to_vec(&new).unwrap(),
                        serde_json::to_vec(&old).unwrap(),
                        "{seed}/{node_count}/{line_count}/{mode}"
                    );
                    assert_eq!(
                        serde_json::to_vec(&(&input.previous, &input.lines, &input.terms)).unwrap(),
                        original
                    );
                }
            }
        }
    }
    assert!(enrich_whiteboard_source_excerpts(None, None, &[], &[]).is_none());
}

#[test]
fn normalization_borrows_unchanged_text_and_exactly_sizes_changed_unicode() {
    for input in ["", "日本語api👩🏽‍💻", "école", "api0", "éÉ😸😸"] {
        let (text, chars) = normalized_excerpt_match_text(input);
        assert!(matches!(text, Cow::Borrowed(_)));
        assert_eq!(text.as_ptr(), input.as_ptr());
        assert_eq!(chars, input.chars().count());
        assert_eq!(text, before::normalized_excerpt_match_text(input));
    }
    for input in [
        "API",
        "ÉCOLE école",
        "\t日本語　 API 👩🏽‍💻\n",
        "\u{2003}\u{a0}\r \u{2028}",
        "Api\u{200b}ＡＢＣ",
    ] {
        let (text, chars) = normalized_excerpt_match_text(input);
        assert_eq!(chars, input.chars().count());
        assert_eq!(text, before::normalized_excerpt_match_text(input));
        let Cow::Owned(text) = text else {
            panic!("changed input should own normalized bytes")
        };
        assert_eq!(text.capacity(), text.len());
    }
}

#[test]
fn term_order_adjacent_duplicates_scalar_scores_and_eight_item_limit_stay_exact() {
    for label in [
        "ab cd ab ef cd ab",
        "abc abc def abc",
        "あ 東京 😸 👩🏽‍💻 あああ école ÉCOLE",
        "のとや・、。，,.:：;；()（）[]【】/／-_+＋=",
        "a bb ccc dddd eeeee ffffff ggggggg hhhhhhhh iiiiiiiii jjjjjjjjjj",
        "API\tapi API api",
    ] {
        let mut node = fixtures::fixture(1, 0, 1).board.nodes.remove(0);
        node.label = label.into();
        node.detail = format!("{label}　日本語");
        let terms = whiteboard_excerpt_terms(&node);
        assert_eq!(
            terms
                .iter()
                .map(|term| term.text.as_ref())
                .collect::<Vec<_>>(),
            before::whiteboard_excerpt_terms(&node)
        );
        for term in &terms {
            assert_eq!(term.char_count, term.text.chars().count());
        }
        assert!(terms.len() <= 8);
    }
    let mut node = fixtures::fixture(1, 0, 1).board.nodes.remove(0);
    node.label = "ab cd ab".into();
    node.detail.clear();
    assert_eq!(
        whiteboard_excerpt_terms(&node)
            .iter()
            .map(|t| t.text.as_ref())
            .collect::<Vec<_>>(),
        ["ab", "cd", "ab"]
    );
}

#[test]
fn excerpt_boundary_and_last_nonempty_duplicate_inheritance_remain_exact() {
    for count in [0, 1, 79, 80, 81, 4096] {
        for unit in ["a", "語", "😸", "👩🏽‍💻"] {
            let input = format!("　\n{}\t ", unit.repeat(count));
            let output = source_excerpt(&input);
            assert_eq!(output, before::clamp_chars(&input, 80));
            assert_eq!(output.capacity(), output.len());
        }
    }
    let mut input = fixtures::fixture(1, 0, 1);
    let node = &mut input.board.nodes[0];
    node.id = "duplicate".into();
    node.label = "same label".into();
    node.source_excerpt.clear();
    let mut one = node.clone();
    one.source_excerpt = " ID first ".into();
    let mut two = one.clone();
    two.source_excerpt = " ID last ".into();
    let mut blank = one.clone();
    blank.source_excerpt = " \t　 ".into();
    let mut label = one.clone();
    label.id = "other".into();
    label.source_excerpt = " label last ".into();
    let previous = fixtures::board(vec![one, two, blank, label]);
    let nodes = &enrich_whiteboard_source_excerpts(Some(input.board), Some(&previous), &[], &[])
        .unwrap()
        .nodes;
    assert_eq!(nodes[0].source_excerpt, "ID last");
}
