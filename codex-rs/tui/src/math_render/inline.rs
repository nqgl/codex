//! Reserve inline image columns before Markdown wrapping, then compose taller rows afterward.
use super::parser;
use crate::terminal_hyperlinks::HyperlinkLine;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use std::collections::HashMap;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

pub(super) struct Prepared {
    pub(super) source: String,
    pub(super) pictures: HashMap<char, Vec<HyperlinkLine>>,
}

pub(super) fn ranges(source: &str) -> Vec<Range<usize>> {
    let mut protected = parser::protected_ranges(source);
    protected.extend(parser::blocks(source));
    let mut result = Vec::new();
    let mut consumed = 0;
    for (start, _) in source.match_indices("\\(") {
        if start < consumed
            || source[..start]
                .bytes()
                .rev()
                .take_while(|ch| *ch == b'\\')
                .count()
                % 2
                != 0
        {
            continue;
        }
        let Some(end) = source[start + 2..]
            .find("\\)")
            .map(|offset| start + 2 + offset + 2)
        else {
            continue;
        };
        if source[start..end].contains('\n')
            || protected
                .iter()
                .any(|range| range.start < end && range.end > start)
        {
            continue;
        }
        result.push(start..end);
        consumed = end;
        if result.len() == 64 {
            break;
        }
    }
    result
}

pub(super) fn prepare(
    source: &str,
    mut picture: impl FnMut(&str) -> Option<Vec<HyperlinkLine>>,
) -> Prepared {
    let mut prepared = Prepared {
        source: String::new(),
        pictures: HashMap::new(),
    };
    let mut markers = ('\u{f000}'..='\u{f8ff}').filter(|ch| !source.contains(*ch));
    let mut start = 0;
    for range in ranges(source) {
        prepared.source.push_str(&source[start..range.start]);
        let raw = &source[range.clone()];
        if let Some(lines) = picture(raw[2..raw.len() - 2].trim())
            && let Some(marker) = markers.next()
        {
            let columns = lines.first().map_or(/*default*/ 0, HyperlinkLine::width);
            prepared.source.extend(std::iter::repeat_n(marker, columns));
            prepared.pictures.insert(marker, lines);
        } else {
            // MathMarkdown preserves the original delimiters when rendering is disabled or
            // unsupported, and provides Unicode fallback while an image is unavailable.
            prepared.source.push_str(raw);
        }
        start = range.end;
    }
    prepared.source.push_str(&source[start..]);
    prepared
}

pub(super) fn expand(
    lines: Vec<HyperlinkLine>,
    pictures: &HashMap<char, Vec<HyperlinkLine>>,
) -> Vec<HyperlinkLine> {
    if pictures.is_empty() {
        return lines;
    }
    let images: HashMap<_, Vec<Vec<Span<'static>>>> = pictures
        .iter()
        .map(|(marker, lines)| {
            (
                *marker,
                lines
                    .iter()
                    .map(|line| {
                        line.line
                            .spans
                            .iter()
                            .flat_map(|span| {
                                span.content
                                    .graphemes(/*is_extended*/ true)
                                    .map(|text| Span::styled(text.to_owned(), span.style))
                            })
                            .collect()
                    })
                    .collect(),
            )
        })
        .collect();
    let mut result = Vec::new();
    let mut used_columns: HashMap<char, usize> = HashMap::new();
    for line in lines {
        let Some(height) = line
            .line
            .spans
            .iter()
            .flat_map(|span| span.content.chars())
            .filter_map(|marker| images.get(&marker).map(Vec::len))
            .max()
        else {
            result.push(line);
            continue;
        };
        let text_row = (height - 1) / 2;
        let mut rows = vec![Vec::new(); height];
        for span in &line.line.spans {
            for grapheme in span.content.graphemes(/*is_extended*/ true) {
                if let Some(marker) = grapheme.chars().next()
                    && let Some(image) = images.get(&marker)
                {
                    let column = used_columns.entry(marker).or_default();
                    let top = (height - image.len()) / 2;
                    for (row, spans) in rows.iter_mut().enumerate() {
                        if let Some(pixel) = row
                            .checked_sub(top)
                            .and_then(|y| image.get(y))
                            .and_then(|pixels| pixels.get(*column))
                        {
                            push_span(spans, &pixel.content, pixel.style);
                        } else {
                            push_span(spans, " ", Style::default());
                        }
                    }
                    *column += 1;
                } else {
                    let padding = " ".repeat(crate::width::display_width(grapheme));
                    for (row, spans) in rows.iter_mut().enumerate() {
                        push_span(
                            spans,
                            if row == text_row { grapheme } else { &padding },
                            if row == text_row {
                                span.style.patch(line.line.style)
                            } else {
                                Style::default()
                            },
                        );
                    }
                }
            }
        }
        for (row, spans) in rows.into_iter().enumerate() {
            // A quote/heading foreground must not replace the RGB image ID in placeholder spans.
            // Preserve the paragraph background, but move text styling onto text spans only.
            let mut composed = Line::from(spans).style(Style {
                bg: line.line.style.bg,
                ..Style::default()
            });
            composed.alignment = line.line.alignment;
            result.push(HyperlinkLine {
                line: composed,
                source: None,
                hyperlinks: if row == text_row {
                    line.hyperlinks.clone()
                } else {
                    Vec::new()
                },
            });
        }
    }
    result
}

fn push_span(spans: &mut Vec<Span<'static>>, text: &str, style: Style) {
    if let Some(last) = spans.last_mut()
        && last.style == style
    {
        last.content.to_mut().push_str(text);
    } else {
        spans.push(Span::styled(text.to_owned(), style));
    }
}

#[cfg(test)]
#[path = "inline_tests.rs"]
mod tests;
