//! Local math rendering. Source stays authoritative; pictures are a bounded UI cache.
//! Unsupported terminals, exhausted budgets, and typesetting failures use the shared math fallback.
mod graphics;
mod inline;
mod parser;
mod renderer;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::terminal_hyperlinks::HyperlinkLine;
use std::collections::HashMap;
use std::io::Write;
use std::io::{self};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::PoisonError;
#[cfg(not(test))]
use std::sync::atomic::AtomicU8;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::mpsc;

const CONFIGURED_MATH_MODE: u8 = 0;
const FORCE_MATH_ON: u8 = 1;
const FORCE_MATH_OFF: u8 = 2;
// `/math` is session-local and overrides resolved settings until this process exits.
#[cfg(not(test))]
static MATH_MODE_OVERRIDE: AtomicU8 = AtomicU8::new(CONFIGURED_MATH_MODE);
#[cfg(test)]
thread_local! {
    static MATH_MODE_OVERRIDE: std::cell::Cell<u8> = const { std::cell::Cell::new(CONFIGURED_MATH_MODE) };
}
static REVISION: AtomicU64 = AtomicU64::new(/*v*/ 0);
static STATE: OnceLock<Mutex<State>> = OnceLock::new();
const MAX_ENTRIES: usize = 128;
const BATCH_SIZE: usize = 8;
const MAX_TRANSFER_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub(super) enum MathStyle {
    Display,
    Inline,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct Key {
    source: String,
    foreground: (u8, u8, u8),
    background: (u8, u8, u8),
    width: usize,
    cell: (u16, u16),
    style: MathStyle,
}

struct State {
    jobs: mpsc::SyncSender<Key>,
    entries: HashMap<Key, Option<Vec<HyperlinkLine>>>,
    transfers: Vec<Vec<u8>>,
    uploaded: usize,
}

pub(crate) fn initialize(events: AppEventSender) {
    // Do not launch subprocesses or emit graphics in tests, multiplexers, or unknown terminals.
    if cfg!(test)
        || !cfg!(target_os = "linux")
        || std::env::var("TERM").as_deref() != Ok("xterm-kitty")
        || ["TMUX", "STY", "ZELLIJ"]
            .iter()
            .any(|key| std::env::var_os(key).is_some())
        || crate::terminal_palette::stdout_color_level()
            != crate::terminal_palette::StdoutColorLevel::TrueColor
        || ![
            "/usr/bin/bwrap",
            "/usr/bin/prlimit",
            "/usr/bin/pdflatex",
            "/usr/bin/pdftoppm",
        ]
        .iter()
        .all(|path| std::path::Path::new(path).is_file())
    {
        return;
    }
    STATE.get_or_init(|| {
        let (jobs, receiver) = mpsc::sync_channel::<Key>(BATCH_SIZE);
        std::thread::spawn(move || {
            let first_id =
                rand::random::<u32>() % (0x00ff_ffff - (MAX_ENTRIES * BATCH_SIZE) as u32) + 1;
            for (batch, first) in receiver.iter().enumerate() {
                for (offset, key) in std::iter::once(first)
                    .chain(receiver.try_iter().take(BATCH_SIZE - 1))
                    .enumerate()
                {
                    let id = first_id + (batch * BATCH_SIZE + offset) as u32;
                    let result = renderer::render(
                        &key.source,
                        key.foreground,
                        key.background,
                        key.style,
                        key.cell.1,
                    )
                    .and_then(|png| graphics::prepare(&png, id, key.width, key.cell));
                    // The receiver cannot get a job until get_or_init has published STATE.
                    let Some(state) = STATE.get() else {
                        break;
                    };
                    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                    if let Some((lines, transfer)) = result
                        && state.transfers.iter().map(Vec::len).sum::<usize>() + transfer.len()
                            <= MAX_TRANSFER_BYTES
                    {
                        state.transfers.push(transfer);
                        state.entries.insert(key, Some(lines));
                    }
                    // Failed entries stay None so replay does not continually retry bad TeX.
                    drop(state);
                }
                REVISION.fetch_add(/*val*/ 1, Ordering::Relaxed);
                events.send(AppEvent::MathRendered);
            }
        });
        Mutex::new(State {
            jobs,
            entries: HashMap::new(),
            transfers: Vec::new(),
            uploaded: 0,
        })
    });
}

pub(crate) fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}

pub(crate) fn apply_mode_override(
    mut rendering: codex_config::types::TuiRendering,
) -> codex_config::types::TuiRendering {
    #[cfg(not(test))]
    let mode = MATH_MODE_OVERRIDE.load(Ordering::Relaxed);
    #[cfg(test)]
    let mode = MATH_MODE_OVERRIDE.get();
    match mode {
        CONFIGURED_MATH_MODE => {}
        FORCE_MATH_ON => rendering.math = true,
        FORCE_MATH_OFF => rendering.math = false,
        _ => unreachable!("math override is assigned only known states"),
    }
    rendering
}

/// Whether the session has math images whose terminal placements can need repair.
pub(crate) fn has_images() -> bool {
    crate::markdown_render::preferences::current().math
        && STATE.get().is_some_and(|state| {
            !state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .transfers
                .is_empty()
        })
}

pub(crate) fn set_mode(mode: &str) -> String {
    let enabled = match mode {
        "on" => true,
        "off" => false,
        _ => !crate::markdown_render::preferences::current().math,
    };
    let mode = if enabled {
        FORCE_MATH_ON
    } else {
        FORCE_MATH_OFF
    };
    #[cfg(not(test))]
    MATH_MODE_OVERRIDE.store(mode, Ordering::Relaxed);
    #[cfg(test)]
    MATH_MODE_OVERRIDE.set(mode);
    let mut rendering = crate::markdown_render::preferences::current();
    rendering.math = enabled;
    crate::markdown_render::preferences::init(rendering);
    REVISION.fetch_add(/*val*/ 1, Ordering::Relaxed);
    if !enabled {
        "Math rendering is off. Equations show their source.".into()
    } else if STATE.get().is_none() {
        "Math rendering is on. Image typesetting needs Linux, direct Kitty with true color, TeX Live, Poppler, and bubblewrap; otherwise equations use Unicode or source.".into()
    } else {
        "Math rendering is on. Completed equations render locally; other equations use Unicode or source.".into()
    }
}

pub(crate) fn flush(writer: &mut impl Write) -> io::Result<()> {
    if let Some(state) = STATE.get() {
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        for transfer in &state.transfers[state.uploaded..] {
            writer.write_all(transfer)?;
        }
        state.uploaded = state.transfers.len();
    }
    Ok(())
}

/// Clear-screen and alternate-screen transitions can discard the terminal's graphics cache.
pub(crate) fn invalidate_images() {
    if let Some(state) = STATE.get() {
        state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .uploaded = 0;
    }
}

pub(crate) fn has_math(source: &str) -> bool {
    (source.contains("\\[") || source.contains("$$")) && !parser::blocks(source).is_empty()
        || source.contains("\\(") && !inline::ranges(source).is_empty()
}

pub(crate) fn render(
    source: &str,
    width: Option<usize>,
    markdown: impl Fn(&str) -> Vec<HyperlinkLine>,
) -> Vec<HyperlinkLine> {
    if !source.contains("\\(") {
        return render_display(source, width, markdown);
    }
    let prepared = inline::prepare(source, |formula| {
        picture(
            formula,
            width.unwrap_or(/*default*/ 80).max(/*other*/ 1),
            MathStyle::Inline,
        )
    });
    inline::expand(
        render_display(&prepared.source, width, markdown),
        &prepared.pictures,
    )
}

fn render_display(
    source: &str,
    width: Option<usize>,
    markdown: impl Fn(&str) -> Vec<HyperlinkLine>,
) -> Vec<HyperlinkLine> {
    if !source.contains("\\[") && !source.contains("$$") {
        return markdown(source);
    }
    let blocks = parser::blocks(source);
    if blocks.is_empty() {
        return markdown(source);
    }
    let width = width.unwrap_or(/*default*/ 80).max(/*other*/ 1);
    // One-cell sentinels preserve Markdown's whole-document reference-link scope, even at
    // width 1. Choose unused characters so source text cannot impersonate a replacement.
    let mut replacements = HashMap::new();
    let mut rewritten = String::new();
    let mut markers = ('\u{e000}'..='\u{f8ff}').filter(|ch| !source.contains(*ch));
    let mut start = 0;
    for block in blocks {
        let raw = source[block.clone()].trim_end();
        let formula = raw[2..raw.len() - 2].trim();
        let Some(picture) = picture(formula, width, MathStyle::Display) else {
            // The shared renderer owns Unicode fallback and raw-source preferences.
            continue;
        };
        let Some(marker) = markers.next() else {
            return markdown(source);
        };
        rewritten.push_str(&source[start..block.start]);
        rewritten.push_str("\n\n");
        rewritten.push(marker);
        rewritten.push_str("\n\n");
        replacements.insert(marker.to_string(), picture);
        start = block.end;
    }
    rewritten.push_str(&source[start..]);
    markdown(&rewritten)
        .into_iter()
        .flat_map(|line| {
            replacements
                .remove(&line.line.to_string())
                .unwrap_or_else(|| vec![line])
        })
        .collect()
}

fn picture(source: &str, width: usize, style: MathStyle) -> Option<Vec<HyperlinkLine>> {
    if !crate::markdown_render::preferences::current().math || !parser::safe_math(source) {
        return None;
    }
    let state = STATE.get()?;
    let window = crossterm::terminal::window_size().ok()?;
    let cell = (
        window.width.checked_div(window.columns)?,
        window.height.checked_div(window.rows)?,
    );
    if cell.0 == 0 || cell.1 == 0 {
        return None;
    }
    let key = Key {
        source: source.into(),
        width,
        cell,
        style,
        foreground: crate::terminal_palette::default_fg()?,
        background: crate::terminal_palette::default_bg()?,
    };
    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(entry) = state.entries.get(&key) {
        return entry.clone();
    }
    // A hard session budget avoids evict/re-render loops during transcript replay. Each raster
    // is at most 512 KiB; resized variants count toward the same 128-entry budget.
    if state.entries.len() < MAX_ENTRIES && state.jobs.try_send(key.clone()).is_ok() {
        state.entries.insert(key, None);
    }
    None
}

#[cfg(test)]
#[path = "math_render_tests.rs"]
mod tests;
