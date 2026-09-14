//! Kitty Unicode placeholders carry their own coordinates through wrapping and scrollback.
use crate::terminal_hyperlinks::HyperlinkLine;
use base64::Engine;
use ratatui::style::Color;
use ratatui::style::Stylize;
use ratatui::text::Line;
use std::io::Write;

// The first 128 entries of Kitty's rowcolumn-diacritics.txt (graphics-protocol specification).
const MARKS: &[char] = &[
    '\u{0305}', '\u{030d}', '\u{030e}', '\u{0310}', '\u{0312}', '\u{033d}', '\u{033e}', '\u{033f}',
    '\u{0346}', '\u{034a}', '\u{034b}', '\u{034c}', '\u{0350}', '\u{0351}', '\u{0352}', '\u{0357}',
    '\u{035b}', '\u{0363}', '\u{0364}', '\u{0365}', '\u{0366}', '\u{0367}', '\u{0368}', '\u{0369}',
    '\u{036a}', '\u{036b}', '\u{036c}', '\u{036d}', '\u{036e}', '\u{036f}', '\u{0483}', '\u{0484}',
    '\u{0485}', '\u{0486}', '\u{0487}', '\u{0592}', '\u{0593}', '\u{0594}', '\u{0595}', '\u{0597}',
    '\u{0598}', '\u{0599}', '\u{059c}', '\u{059d}', '\u{059e}', '\u{059f}', '\u{05a0}', '\u{05a1}',
    '\u{05a8}', '\u{05a9}', '\u{05ab}', '\u{05ac}', '\u{05af}', '\u{05c4}', '\u{0610}', '\u{0611}',
    '\u{0612}', '\u{0613}', '\u{0614}', '\u{0615}', '\u{0616}', '\u{0617}', '\u{0657}', '\u{0658}',
    '\u{0659}', '\u{065a}', '\u{065b}', '\u{065d}', '\u{065e}', '\u{06d6}', '\u{06d7}', '\u{06d8}',
    '\u{06d9}', '\u{06da}', '\u{06db}', '\u{06dc}', '\u{06df}', '\u{06e0}', '\u{06e1}', '\u{06e2}',
    '\u{06e4}', '\u{06e7}', '\u{06e8}', '\u{06eb}', '\u{06ec}', '\u{0730}', '\u{0732}', '\u{0733}',
    '\u{0735}', '\u{0736}', '\u{073a}', '\u{073d}', '\u{073f}', '\u{0740}', '\u{0741}', '\u{0743}',
    '\u{0745}', '\u{0747}', '\u{0749}', '\u{074a}', '\u{07eb}', '\u{07ec}', '\u{07ed}', '\u{07ee}',
    '\u{07ef}', '\u{07f0}', '\u{07f1}', '\u{07f3}', '\u{0816}', '\u{0817}', '\u{0818}', '\u{0819}',
    '\u{081b}', '\u{081c}', '\u{081d}', '\u{081e}', '\u{081f}', '\u{0820}', '\u{0821}', '\u{0822}',
    '\u{0823}', '\u{0825}', '\u{0826}', '\u{0827}', '\u{0829}', '\u{082a}', '\u{082b}', '\u{082c}',
];

pub(super) fn prepare(
    png: &[u8],
    id: u32,
    width: usize,
    cell: (u16, u16),
) -> Option<(Vec<HyperlinkLine>, Vec<u8>)> {
    // Read dimensions without decoding an attacker-influenced raster into an unbounded buffer.
    let reader =
        image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
    let (w, h) = reader.into_dimensions().ok()?;
    if w == 0 || h == 0 || w > 4096 || h > 2048 || cell.0 == 0 || cell.1 == 0 {
        return None;
    }
    let cols = (w as usize)
        .div_ceil(usize::from(cell.0))
        .min(width)
        .clamp(/*min*/ 1, MARKS.len());
    // Rounding columns up must not enlarge a nearly cell-height inline image into two rows.
    let scale = ((cols * usize::from(cell.0)) as f64 / f64::from(w)).min(/*other*/ 1.0);
    let rows = (f64::from(h) * scale / f64::from(cell.1)).ceil() as usize;
    if rows == 0 || rows > 24 {
        return None;
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "Kitty encodes image IDs as 24-bit foreground colors"
    )]
    let color = Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8);
    let lines = MARKS[..rows]
        .iter()
        .map(|row| {
            let mut text = String::new();
            for col in &MARKS[..cols] {
                text.push('\u{10eeee}');
                text.push(*row);
                text.push(*col);
            }
            HyperlinkLine::from(Line::from(text.fg(color)))
        })
        .collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(png);
    let mut transfer = Vec::new();
    let mut chunks = encoded.as_bytes().chunks(4096).peekable();
    let mut first = true;
    while let Some(chunk) = chunks.next() {
        let more = usize::from(chunks.peek().is_some());
        if first {
            write!(
                transfer,
                "\x1b_Ga=T,f=100,q=2,U=1,i={id},c={cols},r={rows},m={more};"
            )
            .ok()?;
        } else {
            write!(transfer, "\x1b_Gm={more};").ok()?;
        }
        transfer.extend_from_slice(chunk);
        transfer.extend_from_slice(b"\x1b\\");
        first = false;
    }
    Some((lines, transfer))
}
