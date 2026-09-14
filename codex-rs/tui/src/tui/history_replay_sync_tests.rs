use super::*;
use pretty_assertions::assert_eq;

#[test]
fn repeated_replays_keep_clear_and_paint_in_one_update() {
    let mut sync = HistoryReplaySync::default();
    let mut bytes = Vec::new();
    sync.finish(&mut bytes).unwrap();
    sync.begin(&mut bytes).unwrap();
    bytes.extend_from_slice(b"CLEAR");
    sync.begin(&mut bytes).unwrap();
    bytes.extend_from_slice(b"IMAGES+HISTORY+COMPOSER");
    sync.finish(&mut bytes).unwrap();
    sync.finish(&mut bytes).unwrap();
    assert_eq!(bytes, b"\x1b[?2026hCLEARIMAGES+HISTORY+COMPOSER\x1b[?2026l");
    sync.begin(&mut bytes).unwrap();
    assert!(sync.active);
}
