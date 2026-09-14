use super::*;
use pretty_assertions::assert_eq;

const BOXED_POLYNOMIAL: &str = r"\boxed{
P(x)=
\underbrace{\prod_{j=1}^{n}\left(x-r_j\right)}_{\text{factored form}}
=
\underbrace{\sum_{k=0}^{n}
c_k x^k}_{\text{polynomial form}}
}";

#[test]
fn boxed_equations_accept_math_but_not_file_access() {
    assert!(parser::safe_math(BOXED_POLYNOMIAL));
    assert!(!parser::safe_math(r"\boxed{\input{/etc/passwd}}"));
}

#[test]
fn wide_equations_use_available_columns_and_shrink_to_fit() {
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(/*w*/ 800, /*h*/ 64)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let layout = [100, 50].map(|width| {
        let (lines, _) =
            graphics::prepare(png.get_ref(), /*id*/ 42, width, /*cell*/ (8, 16)).unwrap();
        lines.iter().map(HyperlinkLine::width).collect::<Vec<_>>()
    });
    insta::assert_debug_snapshot!("wide_math_layout", layout);
}

#[test]
#[ignore = "requires Linux bubblewrap, TeX Live, and Poppler; run explicitly on a capable host"]
fn boxed_polynomial_typesets_locally() {
    let png = renderer::render(
        BOXED_POLYNOMIAL,
        /*fg*/ (235, 235, 235),
        /*bg*/ (56, 56, 56),
        MathStyle::Display,
        /*cell_height*/ 32,
    )
    .expect("boxed equation typesetting failed");
    assert!(
        graphics::prepare(
            &png,
            /*id*/ 42,
            /*width*/ 120,
            /*cell*/ (16, 32)
        )
        .is_some()
    );
}

#[test]
fn display_blocks_exclude_code_and_unclosed_math() {
    let source = "before\n\\[x^2\\]\n```tex\n$$\nx\n$$\n```\n$$\ny\n$$\n\\[\nunclosed";
    assert_eq!(
        parser::blocks(source)
            .iter()
            .map(|range| &source[range.clone()])
            .collect::<Vec<_>>(),
        vec!["\\[x^2\\]\n", "$$\ny\n$$\n"]
    );
    assert!(parser::blocks("Costs $5 or $10. `$$x$$`\n    $$x$$\n").is_empty());
}

#[test]
fn math_vocabulary_is_bounded() {
    for formula in [
        r"A=B^\top\in\mathbb R^{m\times n},\qquad\operatorname{rank}(A)\le n.",
        r"\begin{pmatrix}a&b\\c&d\end{pmatrix}",
        r"\frac{x^2}{\sqrt{1+y}}",
    ] {
        assert!(parser::safe_math(formula), "{formula}");
    }
    for formula in [
        r"\input{/etc/passwd}",
        r"\write18{touch /tmp/oops}",
        r"\csname input\endcsname",
        r"^^5cinput{file}",
        r"\begin{document}oops\end{document}",
        r"\def\x{\x}\x",
        "x%comment",
        "{x",
        "x}",
    ] {
        assert!(!parser::safe_math(formula), "{formula}");
    }
    assert!(!parser::safe_math(&"x".repeat(4097)));
}

#[test]
fn unavailable_rendering_preserves_delimiters_and_markdown() {
    let source = "**Before**\n\n\\[\nx^2+y^2=1\n\\]\n\nAfter.";
    let rendered = crate::markdown::render_markdown_agent_with_links_and_cwd(
        source,
        Some(40),
        /*cwd*/ None,
    );
    let text = rendered
        .iter()
        .map(|line| line.line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!("math_source_fallback", text);
}

#[test]
fn math_does_not_break_reference_links_or_narrow_layouts() {
    let source = "[Before][ref]\n\n$$x$$\n\n[ref]: https://example.com\n";
    let lines = crate::markdown::render_markdown_agent_with_links_and_cwd(
        source,
        Some(40),
        /*cwd*/ None,
    );
    assert!(
        lines
            .iter()
            .flat_map(|line| &line.hyperlinks)
            .any(|link| link.destination == "https://example.com")
    );
    let lines = crate::markdown::render_markdown_agent_with_links_and_cwd(
        "$$x$$",
        Some(1),
        /*cwd*/ None,
    );
    assert_eq!(
        lines
            .iter()
            .map(|line| line.line.to_string())
            .collect::<Vec<_>>(),
        vec!["$", "$", "x", "$", "$"]
    );
}

#[test]
fn math_toggle_invalidates_the_render_cache() {
    let previous = revision();
    assert_eq!(
        set_mode("off"),
        "Math rendering is off. Equations show their source."
    );
    assert!(!ENABLED.load(Ordering::Relaxed));
    assert!(revision() > previous);
    set_mode("toggle");
    assert!(ENABLED.load(Ordering::Relaxed));
}

#[test]
fn placeholders_survive_wrapping_with_explicit_coordinates() {
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(/*w*/ 80, /*h*/ 32)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let (lines, transfer) = graphics::prepare(
        png.get_ref(),
        /*id*/ 42,
        /*width*/ 20,
        /*cell*/ (8, 16),
    )
    .unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines.iter().map(HyperlinkLine::width).collect::<Vec<_>>(),
        vec![10, 10]
    );
    assert!(
        lines[1].line.spans[0]
            .content
            .starts_with("\u{10eeee}\u{030d}\u{0305}")
    );
    let transfer = String::from_utf8(transfer).unwrap();
    assert!(transfer.starts_with("\x1b_Ga=T,f=100,q=2,U=1,i=42,c=10,r=2,m=0;"));
    assert!(transfer.ends_with("\x1b\\"));
    let (narrow, _) = graphics::prepare(
        png.get_ref(),
        /*id*/ 43,
        /*width*/ 5,
        /*cell*/ (8, 16),
    )
    .unwrap();
    assert_eq!(
        narrow.iter().map(HyperlinkLine::width).collect::<Vec<_>>(),
        vec![5]
    );
}

#[test]
#[ignore = "requires Linux bubblewrap, TeX Live, and Poppler; run explicitly on a capable host"]
fn local_typesetting_is_sandboxed() {
    let png = renderer::render(
        r"A=B^\top\in\mathbb R^{m\times n},\qquad\operatorname{rank}(A)\le n.",
        /*fg*/ (235, 235, 235),
        /*bg*/ (56, 56, 56),
        MathStyle::Display,
        /*cell_height*/ 16,
    )
    .expect("sandboxed typesetting failed");
    assert!(
        graphics::prepare(
            &png,
            /*id*/ 42,
            /*width*/ 80,
            /*cell*/ (8, 16)
        )
        .is_some()
    );
}
