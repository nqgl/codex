//! Locate standalone display math before Markdown consumes backslash delimiters.

use pulldown_cmark::Event;
use pulldown_cmark::Parser;
use pulldown_cmark::Tag;
use pulldown_cmark::TagEnd;
use std::ops::Range;

pub(super) fn protected_ranges(source: &str) -> Vec<Range<usize>> {
    let mut protected = Vec::new();
    let mut code_start = None;
    for (event, range) in Parser::new(source).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => code_start = Some(range.start),
            Event::End(TagEnd::CodeBlock) => {
                if let Some(start) = code_start.take() {
                    protected.push(start..range.end);
                }
            }
            Event::Code(_)
            | Event::Html(_)
            | Event::InlineHtml(_)
            | Event::Start(Tag::Link { .. } | Tag::Image { .. }) => protected.push(range),
            _ => {}
        }
    }
    protected
}

pub(super) fn blocks(source: &str) -> Vec<Range<usize>> {
    let protected = protected_ranges(source);
    let mut result = Vec::new();
    let mut open: Option<(usize, &str)> = None;
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let text = line.trim_end_matches(['\r', '\n']);
        if protected.iter().any(|range| range.contains(&offset)) {
            open = None;
        } else if let Some((start, close)) = open {
            if text == close {
                result.push(start..offset + line.len());
                open = None;
            }
        } else if text == "\\[" || text == "$$" {
            open = Some((offset, if text == "$$" { "$$" } else { "\\]" }));
        } else if (text.starts_with("\\[") && text.ends_with("\\]"))
            || (text.starts_with("$$") && text.ends_with("$$") && text.len() > 4)
        {
            result.push(offset..offset + line.len());
        }
        offset += line.len();
        if result.len() >= 32 {
            break;
        }
    }
    result
}

/// TeX is a programming language. Only a small math vocabulary is accepted, even inside the
/// filesystem/network sandbox. Unknown commands retain their original source instead.
pub(super) fn safe_math(source: &str) -> bool {
    if source.len() > 4096 || source.is_empty() || !source.is_ascii() {
        return false;
    }
    let mut chars = source.chars().peekable();
    let mut depth = 0_u32;
    let mut commands = 0;
    while let Some(ch) = chars.next() {
        match ch {
            '{' => {
                depth += 1;
                if depth > 24 {
                    return false;
                }
            }
            '}' => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            '%' | '#' | '$' | '\0' | '\u{1b}' => return false,
            '^' if chars.peek() == Some(&'^') => return false,
            '\\' => {
                commands += 1;
                if commands > 256 {
                    return false;
                }
                let mut command = String::new();
                while let Some(&ch) = chars.peek() {
                    if !ch.is_ascii_alphabetic() {
                        break;
                    }
                    command.push(ch);
                    chars.next();
                }
                if command.is_empty() {
                    if !matches!(
                        chars.next(),
                        Some(',' | ';' | ':' | '!' | ' ' | '{' | '}' | '|' | '\\')
                    ) {
                        return false;
                    }
                } else if command == "begin" || command == "end" {
                    if chars.next() != Some('{') {
                        return false;
                    }
                    let mut environment = String::new();
                    let mut closed = false;
                    for ch in chars.by_ref() {
                        if ch == '}' {
                            closed = true;
                            break;
                        }
                        environment.push(ch);
                    }
                    if !closed
                        || !matches!(
                            environment.as_str(),
                            "matrix"
                                | "pmatrix"
                                | "bmatrix"
                                | "Bmatrix"
                                | "vmatrix"
                                | "Vmatrix"
                                | "cases"
                                | "aligned"
                                | "gathered"
                        )
                    {
                        return false;
                    }
                } else if !ALLOWED.split_whitespace().any(|allowed| allowed == command) {
                    return false;
                }
            }
            ch if ch.is_control() && !matches!(ch, '\n' | '\r' | '\t') => return false,
            _ => {}
        }
    }
    depth == 0
}

const ALLOWED: &str = "boxed frac dfrac tfrac sqrt binom dbinom tbinom overline underline underbrace overbrace
hat widehat bar vec dot ddot tilde widetilde acute grave breve check
sum prod coprod int iint iiint oint lim limsup liminf min max sup inf det gcd log ln exp sin cos tan cot sec csc sinh cosh tanh arcsin arccos arctan
alpha beta gamma delta epsilon varepsilon zeta eta theta vartheta iota kappa lambda mu nu xi pi varpi rho varrho sigma varsigma tau upsilon phi varphi chi psi omega
Gamma Delta Theta Lambda Xi Pi Sigma Upsilon Phi Psi Omega
mathbb mathcal mathrm mathbf mathit mathsf mathtt boldsymbol operatorname text textrm textbf
left right middle big Big bigg Bigg bigl bigr Bigl Bigr biggl biggr Biggl Biggr
quad qquad thinspace negthinspace displaystyle textstyle scriptstyle scriptscriptstyle
times cdot div pm mp ast star circ bullet cap cup uplus sqcap sqcup wedge vee setminus oplus otimes odot
le leq ge geq ne neq equiv approx sim simeq cong propto ll gg subset supset subseteq supseteq in notin ni mid parallel perp models vdash dashv
to mapsto rightarrow leftarrow leftrightarrow Rightarrow Leftarrow Leftrightarrow longrightarrow longleftarrow Longrightarrow Longleftarrow implies iff
infty partial nabla forall exists neg lnot emptyset varnothing ell hbar Re Im top bot angle triangle lceil rceil lfloor rfloor langle rangle lbrace rbrace vert Vert
ldots cdots vdots ddots dots not mod bmod pmod limits nolimits substack underset overset";
