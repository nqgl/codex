use super::*;
use pretty_assertions::assert_eq;
use ratatui::style::Stylize;

#[test]
fn paragraph_foregrounds_do_not_overwrite_image_identifiers() {
    let prepared = prepare("quote \\(x\\) end", |_| {
        Some(vec![Line::from("IMG".red()).into()])
    });
    let mut lines = crate::markdown_render::render_markdown_lines_with_width_and_cwd(
        &prepared.source,
        Some(40),
        /*cwd*/ None,
    );
    let text_style = Style::default().green().underlined();
    lines[0].line.style = text_style;
    let expanded = expand(lines, &prepared.pictures);
    assert_eq!(
        expanded,
        vec![HyperlinkLine::from(Line::from(vec![
            Span::styled("quote ", text_style),
            "IMG".red(),
            Span::styled(" end", text_style)
        ]))]
    );
}

#[test]
fn inline_ranges_respect_code_links_displays_and_unfinished_input() {
    let source = "Here \\(u_t\\). `\\(code\\)`\n```tex\n\\(code\\)\n```\n\\[\n\\text{\\(display\\)}\n\\]\n[\\(label\\)](https://example.com)\n\\(unfinished";
    assert_eq!(
        ranges(source)
            .iter()
            .map(|range| &source[range.clone()])
            .collect::<Vec<_>>(),
        vec!["\\(u_t\\)"]
    );
}

#[test]
fn unsupported_inline_math_keeps_delimiters_and_subscripts() {
    let source = "**Only** \\(m/\\sqrt{as+V}\\), with \\(V\\).";
    let lines = crate::markdown::render_markdown_agent_with_links_and_cwd(
        source,
        Some(60),
        /*cwd*/ None,
    );
    insta::assert_snapshot!(
        "inline_math_source",
        lines
            .iter()
            .map(|line| line.line.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );
    let streamed = crate::markdown::render_streaming_markdown_agent_with_links_and_cwd(
        source,
        Some(60),
        /*cwd*/ None,
    );
    assert_eq!(streamed.lines, lines);
}

#[test]
fn inline_images_expand_rows_without_losing_prose_or_links() {
    let source = "**Before** \\(x\\) and \\(y\\); [link][ref].\n\n[ref]: https://example.com\n";
    let prepared = prepare(source, |formula| {
        Some(if formula == "x" {
            vec![Line::from("[x]").into(), Line::from("/x\\").into()]
        } else {
            vec![Line::from("[y]").into()]
        })
    });
    let lines = crate::markdown_render::render_markdown_lines_with_width_and_cwd(
        &prepared.source,
        Some(40),
        /*cwd*/ None,
    );
    let links = lines
        .iter()
        .flat_map(|line| line.hyperlinks.clone())
        .collect::<Vec<_>>();
    let expanded = expand(lines, &prepared.pictures);
    assert_eq!(
        expanded
            .iter()
            .flat_map(|line| line.hyperlinks.clone())
            .collect::<Vec<_>>(),
        links
    );
    insta::assert_snapshot!(
        "inline_math_layout",
        expanded
            .iter()
            .map(|line| line.line.to_string().trim_end().to_owned())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn inline_image_wraps_as_a_word_and_preserves_its_columns_when_clipped() {
    let prepared = prepare("abcd \\(x\\) end", |_| Some(vec![Line::from("[x]").into()]));
    for width in [6, 2] {
        let lines = crate::markdown_render::render_markdown_lines_with_width_and_cwd(
            &prepared.source,
            Some(width),
            /*cwd*/ None,
        );
        let expanded = expand(lines, &prepared.pictures);
        assert!(expanded.iter().all(|line| line.width() <= width));
        let text = expanded
            .iter()
            .map(|line| line.line.to_string())
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(text.matches("[x]").count(), 1);
    }
}

#[test]
#[ignore = "requires Linux bubblewrap, TeX Live, and Poppler"]
fn inline_tex_uses_text_sized_rasters() {
    for source in [r"u_t", r"m/\sqrt{as+V}"] {
        let png = crate::math_render::renderer::render(
            source,
            /*fg*/ (235, 235, 235),
            /*bg*/ (56, 56, 56),
            crate::math_render::MathStyle::Inline,
            /*cell_height*/ 40,
        )
        .unwrap();
        let (lines, _) = crate::math_render::graphics::prepare(
            &png,
            /*id*/ 42,
            /*width*/ 100,
            /*cell*/ (16, 40),
        )
        .unwrap();
        assert!(lines.len() <= 2);
    }
}
