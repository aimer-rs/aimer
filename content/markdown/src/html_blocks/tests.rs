use aimer_rubick::ShareRef;

use crate::document::{Alignment, Block, Document, Inline, ListItem, TableRow};

fn blocks(source: &str) -> Vec<Block> {
    Document::parse(source).unwrap_or_else(|error| panic!("{source:?}: {error}")).blocks
}

fn text(text: &str) -> Inline {
    Inline::Text(ShareRef::from(text.to_owned()))
}

fn paragraph(inlines: Vec<Inline>) -> Block {
    Block::Paragraph(inlines)
}

fn image(url: &str, alt: &str) -> Inline {
    Inline::Image { url: ShareRef::from(url.to_owned()), title: None, alt: ShareRef::from(alt.to_owned()) }
}

// ---- inline

#[test]
fn bold_italic_code_keys_and_strikethrough_tags_are_the_inlines_that_markdown_makes_of_them() {
    assert_eq!(
        blocks("a <b>bold</b> and <i>it</i> and <code>c</code> <kbd>Ctrl</kbd> <s>x</s> <strong>s</strong> <em>e</em>"),
        [paragraph(vec![
            text("a "),
            Inline::Strong(vec![text("bold")]),
            text(" and "),
            Inline::Emphasis(vec![text("it")]),
            text(" and "),
            Inline::Code("c".into()),
            text(" "),
            Inline::Code("Ctrl".into()),
            text(" "),
            Inline::Delete(vec![text("x")]),
            text(" "),
            Inline::Strong(vec![text("s")]),
            text(" "),
            Inline::Emphasis(vec![text("e")]),
        ])]
    );
}

#[test]
fn a_line_break_tag_is_a_hard_break_in_any_spelling() {
    for source in ["one<br>two", "one<br/>two", "one<br />two", "one<BR>two"] {
        assert_eq!(blocks(source), [paragraph(vec![text("one"), Inline::HardBreak, text("two")])], "{source}");
    }
}

#[test]
fn a_link_has_its_address_its_title_and_what_is_in_it() {
    assert_eq!(
        blocks(r#"see <a href="https://x.dev?a=1&amp;b=2" title="T">the <b>site</b></a>."#),
        [paragraph(vec![
            text("see "),
            Inline::Link {
                url: "https://x.dev?a=1&b=2".into(),
                title: Some("T".into()),
                content: vec![text("the "), Inline::Strong(vec![text("site")])],
            },
            text("."),
        ])]
    );
    assert_eq!(
        blocks(r#"<a name="top">anchor</a>"#),
        [paragraph(vec![text("anchor")])],
        "an anchor with nowhere to go is its words"
    );
}

#[test]
fn an_image_tag_is_an_image_with_its_address_and_its_words_and_its_title() {
    assert_eq!(
        blocks(r#"<img src="a.png" alt="A logo" title="Hello">"#),
        [paragraph(vec![Inline::Image { url: "a.png".into(), title: Some("Hello".into()), alt: "A logo".into() }])]
    );
    assert_eq!(blocks(r#"<img src="b.png">"#), [paragraph(vec![image("b.png", "")])]);
    assert_eq!(blocks(r#"x <img src=c.png alt=C> y"#), [paragraph(vec![text("x "), image("c.png", "C"), text(" y")])]);
    assert_eq!(blocks("<img>"), [], "an image of nothing is nothing");
}

#[test]
fn markdown_inside_html_tags_is_still_markdown() {
    assert_eq!(
        blocks("<b>*em* and `code`</b>"),
        [paragraph(vec![Inline::Strong(vec![Inline::Emphasis(vec![text("em")]), text(" and "), Inline::Code("code".into())])])]
    );
    assert_eq!(blocks("**<kbd>Esc</kbd>**"), [paragraph(vec![Inline::Strong(vec![Inline::Code("Esc".into())])])]);
}

#[test]
fn tags_that_are_not_closed_are_closed_where_the_paragraph_ends_and_end_tags_that_close_nothing_are_passed_over() {
    assert_eq!(blocks("a <b>bold"), [paragraph(vec![text("a "), Inline::Strong(vec![text("bold")])])]);
    assert_eq!(blocks("a</b> b"), [paragraph(vec![text("a"), text(" b")])]);
    assert_eq!(
        blocks("<b><i>x</b> y</i>"),
        [paragraph(vec![Inline::Strong(vec![Inline::Emphasis(vec![text("x")])]), text(" y")])],
        "an end tag closes what is inside the tag it names too"
    );
}

#[test]
fn tags_that_mean_nothing_here_are_passed_over_and_what_is_in_them_is_kept() {
    assert_eq!(
        blocks(r#"<span class="x">hi</span> x<sup>2</sup> <u>u</u> <font color="red">f</font>"#),
        [paragraph(vec![text("hi"), text(" x"), text("2"), text(" "), text("u"), text(" "), text("f")])]
    );
}

#[test]
fn entities_in_html_text_are_the_characters_they_stand_for() {
    assert_eq!(blocks("<b>a &amp; b &lt;c&gt; &copy;</b>"), [paragraph(vec![Inline::Strong(vec![text("a & b <c> \u{a9}")])])]);
}

#[test]
fn script_style_and_embedding_elements_are_errors_and_not_dropped() {
    for source in ["<script>alert('no')</script>", "<style>p { color: red }</style>", "a <iframe src=\"x\"></iframe>", "<object data=\"x\"></object>", "<embed src=\"x\">"] {
        let error = Document::parse(source).expect_err(source);
        assert!(error.to_string().contains("HTML"), "{source}: {error}");
    }
}

// ---- blocks

#[test]
fn a_centered_paragraph_of_images_is_aligned_blocks() {
    assert_eq!(
        blocks(r#"<p align="center"><img src="logo.png" alt="Logo"></p>"#),
        [Block::Aligned { alignment: Alignment::Center, blocks: vec![paragraph(vec![image("logo.png", "Logo")])] }]
    );
}

#[test]
fn a_div_and_a_center_tag_and_a_style_align_what_is_in_them() {
    let centered = |blocks| Block::Aligned { alignment: Alignment::Center, blocks };
    assert_eq!(
        blocks("<div align=\"center\">\n<h1>Title</h1>\n<p>text <b>x</b></p>\n</div>"),
        [centered(vec![Block::Heading { depth: 1, content: vec![text("Title")] }, paragraph(vec![text("text "), Inline::Strong(vec![text("x")])])])]
    );
    assert_eq!(blocks("<center>middle</center>"), [centered(vec![paragraph(vec![text("middle")])])]);
    assert_eq!(
        blocks(r#"<p style="color: red; text-align: right ;">r</p>"#),
        [Block::Aligned { alignment: Alignment::Right, blocks: vec![paragraph(vec![text("r")])] }]
    );
    assert_eq!(blocks(r#"<p align="left">l</p>"#), [paragraph(vec![text("l")])], "left is where it is already");
    assert_eq!(blocks("<div>plain</div>"), [paragraph(vec![text("plain")])]);
}

#[test]
fn a_heading_tag_is_a_heading_and_keeps_its_alignment() {
    assert_eq!(blocks("<h3>Three</h3>"), [Block::Heading { depth: 3, content: vec![text("Three")] }]);
    assert_eq!(
        blocks(r#"<h1 align="center">Title <i>x</i></h1>"#),
        [Block::Aligned { alignment: Alignment::Center, blocks: vec![Block::Heading { depth: 1, content: vec![text("Title "), Inline::Emphasis(vec![text("x")])] }] }]
    );
}

#[test]
fn markdown_between_the_tags_of_a_block_is_in_the_block() {
    assert_eq!(
        blocks("<div align=\"center\">\n\n# Title\n\nSome *text*\n\n</div>\n\nAfter"),
        [
            Block::Aligned {
                alignment: Alignment::Center,
                blocks: vec![Block::Heading { depth: 1, content: vec![text("Title")] }, paragraph(vec![text("Some "), Inline::Emphasis(vec![text("text")])])],
            },
            paragraph(vec![text("After")]),
        ]
    );
}

#[test]
fn details_have_a_summary_and_blocks_and_are_shut_unless_they_say_open() {
    let hidden = blocks("<details>\n<summary>More <b>info</b></summary>\n\nHidden **text**\n\n</details>");
    assert_eq!(
        hidden,
        [Block::Details {
            summary: vec![text("More "), Inline::Strong(vec![text("info")])],
            open: false,
            blocks: vec![paragraph(vec![text("Hidden "), Inline::Strong(vec![text("text")])])],
        }]
    );
    let shown = blocks("<details open>\n<summary>S</summary>\n\nBody\n\n</details>");
    assert!(matches!(&shown[..], [Block::Details { open: true, .. }]));
    let nameless = blocks("<details>\n\nBody\n\n</details>");
    assert_eq!(
        nameless,
        [Block::Details { summary: vec![text("Details")], open: false, blocks: vec![paragraph(vec![text("Body")])] }],
        "no summary is the word a browser shows"
    );
}

#[test]
fn a_table_has_rows_of_cells_and_the_alignment_of_its_first_row() {
    assert_eq!(
        blocks("<table>\n<thead><tr><th>A</th><th align=\"right\">B</th></tr></thead>\n<tbody>\n<tr><td>1</td><td>2<br>x</td></tr>\n</tbody>\n</table>"),
        [Block::Table {
            alignments: vec![Alignment::None, Alignment::Right],
            rows: vec![
                TableRow { cells: vec![vec![text("A")], vec![text("B")]] },
                TableRow { cells: vec![vec![text("1")], vec![text("2"), Inline::HardBreak, text("x")]] },
            ],
        }]
    );
}

#[test]
fn the_cells_of_a_table_may_have_images_and_links_and_end_tags_may_be_left_out() {
    assert_eq!(
        blocks("<table><tr><td><a href=\"u\">l</a><td><img src=\"i.png\" alt=\"I\"><tr><td>x<td>y</table>"),
        [Block::Table {
            alignments: vec![Alignment::None, Alignment::None],
            rows: vec![
                TableRow { cells: vec![vec![Inline::Link { url: "u".into(), title: None, content: vec![text("l")] }], vec![image("i.png", "I")]] },
                TableRow { cells: vec![vec![text("x")], vec![text("y")]] },
            ],
        }]
    );
}

#[test]
fn lists_have_items_and_an_ordered_one_may_start_at_a_number() {
    assert_eq!(
        blocks("<ul>\n<li>one</li>\n<li>two <b>b</b></li>\n</ul>"),
        [Block::List {
            ordered: false,
            start: None,
            items: vec![
                ListItem { checked: None, blocks: vec![paragraph(vec![text("one")])] },
                ListItem { checked: None, blocks: vec![paragraph(vec![text("two "), Inline::Strong(vec![text("b")])])] },
            ],
        }]
    );
    let ordered = blocks("<ol start=\"3\"><li>c<li>d</ol>");
    assert!(matches!(&ordered[..], [Block::List { ordered: true, start: Some(3), items }] if items.len() == 2));
}

#[test]
fn a_rule_a_quote_and_preformatted_code_are_the_blocks_markdown_makes_of_them() {
    assert_eq!(blocks("<hr>"), [Block::ThematicBreak]);
    assert_eq!(blocks("<blockquote>q <b>x</b></blockquote>"), [Block::Blockquote(vec![paragraph(vec![text("q "), Inline::Strong(vec![text("x")])])])]);
    assert_eq!(
        blocks("<pre><code class=\"language-rust\">fn main() {\n    a &lt; b;\n}\n</code></pre>"),
        [Block::Code { value: "fn main() {\n    a < b;\n}\n".into(), language: Some("rust".into()), meta: None }]
    );
    assert_eq!(
        blocks("<pre>  keep   spaces\n</pre>"),
        [Block::Code { value: "  keep   spaces\n".into(), language: None, meta: None }],
        "what is preformatted is not collapsed"
    );
}

#[test]
fn text_and_inline_tags_between_blocks_are_paragraphs_and_blank_text_and_comments_are_nothing() {
    assert_eq!(blocks("<div>Hello <b>x</b><p>para</p>tail</div>"), [paragraph(vec![text("Hello "), Inline::Strong(vec![text("x")])]), paragraph(vec![text("para")]), paragraph(vec![text("tail")])]);
    assert_eq!(blocks("<!-- a comment -->"), []);
    assert_eq!(blocks("<div>\n  \n</div>"), []);
    assert_eq!(blocks("<p>\n  spaced\n  out\n</p>"), [paragraph(vec![text("spaced out")])], "line breaks in HTML are blanks");
}

#[test]
fn what_is_left_open_is_closed_where_the_document_ends() {
    assert_eq!(
        blocks("<div align=\"center\">\n\ntext"),
        [Block::Aligned { alignment: Alignment::Center, blocks: vec![paragraph(vec![text("text")])] }]
    );
    assert_eq!(blocks("</div>\n\nafter"), [paragraph(vec![text("after")])], "an end tag that closes nothing");
    assert!(matches!(&blocks("<details>\n\nbody")[..], [Block::Details { .. }]));
}

#[test]
fn html_in_a_quote_a_list_and_between_markdown_is_converted_where_it_is() {
    assert_eq!(blocks("> <b>x</b>"), [Block::Blockquote(vec![paragraph(vec![Inline::Strong(vec![text("x")])])])]);
    assert_eq!(
        blocks("- <kbd>a</kbd> item\n- two"),
        [Block::List {
            ordered: false,
            start: None,
            items: vec![
                ListItem { checked: None, blocks: vec![paragraph(vec![Inline::Code("a".into()), text(" item")])] },
                ListItem { checked: None, blocks: vec![paragraph(vec![text("two")])] },
            ],
        }]
    );
    assert_eq!(
        blocks("# Title\n\n<img src=\"a.png\" alt=\"A\">\n\ntext"),
        [Block::Heading { depth: 1, content: vec![text("Title")] }, paragraph(vec![image("a.png", "A")]), paragraph(vec![text("text")])]
    );
}

#[test]
fn a_document_without_html_is_what_it_was() {
    assert_eq!(blocks("# T\n\nplain *text*"), [Block::Heading { depth: 1, content: vec![text("T")] }, paragraph(vec![text("plain "), Inline::Emphasis(vec![text("text")])])]);
    let _: ShareRef<str> = "unused".into();
}
