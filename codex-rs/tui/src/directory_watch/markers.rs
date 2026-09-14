//! Marker state and chained hashtag metadata for tagged directory watches.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

use super::DirectoryWatchUrgency;

const HEADER_MAX_BYTES: usize = 8 * 1024;
const HEADER_MAX_LINES: usize = 8;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_INITIAL_SCAN_BYTES: usize = 64 * 1024 * 1024;
const MAX_INITIAL_SCAN_ENTRIES: usize = 10_000;
const MAX_MATCHED_TAGS: usize = 20;
const MAX_TAG_CHARS: usize = 300;
pub(super) const MAX_BATCH_SCAN_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TagOccurrence {
    canonical: String,
    pub(super) text: String,
    pub(super) urgency: DirectoryWatchUrgency,
    watched: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StoredOccurrence {
    count: usize,
    text: String,
    urgency: DirectoryWatchUrgency,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct MarkerSnapshot {
    occurrences: HashMap<String, StoredOccurrence>,
    watched: Vec<TagOccurrence>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum MarkerChange {
    Relevant {
        urgency: DirectoryWatchUrgency,
        tags: Vec<String>,
    },
    Irrelevant,
    Uninspected,
}

pub(super) struct MarkerTracker {
    markers: Vec<String>,
    snapshots: HashMap<PathBuf, MarkerSnapshot>,
}

impl MarkerTracker {
    pub(super) fn empty(markers: Vec<String>) -> Self {
        Self {
            markers,
            snapshots: HashMap::new(),
        }
    }

    pub(super) fn initial(root: &Path, git_dir: Option<&Path>, markers: Vec<String>) -> Self {
        let mut tracker = Self::empty(markers);
        let mut directories = VecDeque::from([root.to_path_buf()]);
        let mut inspected_entries = 0;
        let mut remaining_bytes = MAX_INITIAL_SCAN_BYTES;

        while let Some(directory) = directories.pop_front() {
            let Ok(entries) = std::fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                if inspected_entries >= MAX_INITIAL_SCAN_ENTRIES || remaining_bytes == 0 {
                    return tracker;
                }
                inspected_entries += 1;
                let path = entry.path();
                if git_dir.is_some_and(|git_dir| path.starts_with(git_dir)) {
                    continue;
                }
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_dir() {
                    directories.push_back(path);
                    continue;
                }
                if file_type.is_file()
                    && let Some(snapshot) =
                        read_snapshot(&path, &tracker.markers, &mut remaining_bytes)
                    && !snapshot.occurrences.is_empty()
                {
                    tracker.snapshots.insert(path, snapshot);
                }
            }
        }
        tracker
    }

    pub(super) fn inspect(&mut self, path: &Path, remaining_bytes: &mut usize) -> MarkerChange {
        let Some(current) = read_snapshot(path, &self.markers, remaining_bytes) else {
            return MarkerChange::Uninspected;
        };
        let previous = self.snapshots.get(path).cloned().unwrap_or_default();
        let mut matched = BTreeMap::new();
        let mut urgency = None;

        for (canonical, occurrence) in &current.occurrences {
            let previous_count = previous
                .occurrences
                .get(canonical)
                .map(|occurrence| occurrence.count)
                .unwrap_or(0);
            if occurrence.count > previous_count {
                matched.insert(canonical.clone(), occurrence.text.clone());
                urgency = urgency.max(Some(occurrence.urgency));
            }
        }
        for occurrence in current.watched.iter().chain(&previous.watched) {
            matched.insert(occurrence.canonical.clone(), occurrence.text.clone());
            urgency = urgency.max(Some(occurrence.urgency));
        }

        if current.occurrences.is_empty() {
            self.snapshots.remove(path);
        } else {
            self.snapshots.insert(path.to_path_buf(), current);
        }
        match urgency {
            Some(urgency) => MarkerChange::Relevant {
                urgency,
                tags: matched.into_values().take(MAX_MATCHED_TAGS).collect(),
            },
            None => MarkerChange::Irrelevant,
        }
    }
}

fn read_snapshot(
    path: &Path,
    markers: &[String],
    remaining_bytes: &mut usize,
) -> Option<MarkerSnapshot> {
    if !path.exists() {
        return Some(MarkerSnapshot::default());
    }
    if *remaining_bytes == 0 {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let byte_limit = (*remaining_bytes).min(MAX_FILE_BYTES as usize);
    let mut bytes = Vec::new();
    file.take(byte_limit as u64).read_to_end(&mut bytes).ok()?;
    *remaining_bytes = remaining_bytes.saturating_sub(bytes.len());

    let contents = String::from_utf8_lossy(&bytes);
    let mut occurrences: HashMap<String, StoredOccurrence> = HashMap::new();
    for line in contents.lines() {
        for occurrence in tags_in_line(line, markers) {
            let stored = occurrences
                .entry(occurrence.canonical.clone())
                .or_insert_with(|| StoredOccurrence {
                    count: 0,
                    text: occurrence.text.clone(),
                    urgency: occurrence.urgency,
                });
            stored.count += 1;
            stored.urgency = stored.urgency.max(occurrence.urgency);
        }
    }

    let header_end = contents.floor_char_boundary(HEADER_MAX_BYTES.min(contents.len()));
    let watched = contents[..header_end]
        .lines()
        .take(HEADER_MAX_LINES)
        .flat_map(|line| tags_in_line(line, markers))
        .filter(|occurrence| occurrence.watched)
        .collect();
    Some(MarkerSnapshot {
        occurrences,
        watched,
    })
}

pub(super) fn tags_in_line(line: &str, markers: &[String]) -> Vec<TagOccurrence> {
    let lower = line.to_ascii_lowercase();
    let urgency = if lower.contains("#urgency:high") {
        DirectoryWatchUrgency::High
    } else {
        DirectoryWatchUrgency::Low
    };
    let mut occurrences = Vec::new();
    for marker in markers {
        for (start, _) in lower.match_indices(marker) {
            let marker_end = start + marker.len();
            if lower[marker_end..]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-'))
            {
                continue;
            }
            let suffix = &line[start..];
            let end = suffix
                .char_indices()
                .find_map(|(index, ch)| {
                    (ch.is_whitespace() || matches!(ch, ',' | ';' | ')' | ']' | '}'))
                        .then_some(index)
                })
                .unwrap_or(suffix.len());
            let mut text = suffix[..end]
                .trim_end_matches(['.', '!', '?', ':'])
                .to_string();
            if text.chars().count() > MAX_TAG_CHARS {
                text = text.chars().take(MAX_TAG_CHARS - 1).collect();
                text.push('…');
            }
            let canonical = text.to_ascii_lowercase();
            let watched = canonical
                .split('#')
                .skip(1)
                .any(|metadata| metadata == "watch");
            occurrences.push(TagOccurrence {
                canonical,
                text,
                urgency,
                watched,
            });
        }
    }
    occurrences
}
