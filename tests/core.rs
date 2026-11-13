use clipwheel::settings::Hotkey;
use clipwheel::{geometry::*, model::*};

#[test]
fn shortcuts_roundtrip_and_reject_ambiguous_keys() {
    let key = Hotkey::parse(" ctrl + shift + q ").unwrap();
    assert_eq!(key.label(), "Ctrl+Shift+Q");
    assert_eq!(Hotkey::parse(&key.label()), Some(key));
    for text in ["V", "Ctrl", "Ctrl+V+Q", "Win+V", "Ctrl+F12", "Ctrl++"] {
        assert!(Hotkey::parse(text).is_none(), "{text}");
    }
}

#[test]
fn image_previews_are_bounded_and_accounted() {
    let p = Prepared::new(Payload::Image {
        width: 1024,
        height: 512,
        bgra: vec![255; 1024 * 512 * 4],
    });
    let thumb = p.thumbnail.as_ref().expect("image preview");
    assert_eq!((thumb.width, thumb.height), (512, 256));
    assert_eq!(thumb.bgra.len(), 512 * 256 * 4);
    let mut h = History::new(32, 10 * 1024 * 1024, 10 * 1024 * 1024);
    h.insert_prepared(p).unwrap();
    assert!(h.bytes >= 1024 * 512 * 4 + 512 * 256 * 4);
}

#[test]
fn large_unicode_details_are_complete_in_bounded_sections() {
    let text = "A🦀ไทย\n".repeat(3000);
    let mut h = History::new(32, 1024 * 1024, 1024 * 1024);
    h.insert(Payload::Text(text.clone())).unwrap();
    let e = &h.entries[0];
    assert!(e.section_count() > 1);
    let mut reconstructed = String::new();
    for i in 0..e.section_count() {
        let s = e.section(i);
        assert!(s.len() <= 4096);
        reconstructed.push_str(s);
    }
    assert_eq!(reconstructed, text);
}

#[test]
fn pinned_snapshot_and_new_copies_share_one_memory_budget() {
    let mut h = History::new(2, 200, 200);
    h.insert(Payload::Text("a".repeat(40))).unwrap();
    let pinned = h.entries.clone();
    h.insert_pinned(Prepared::new(Payload::Text("b".repeat(40))), &pinned)
        .unwrap();
    h.insert_pinned(Prepared::new(Payload::Text("c".repeat(40))), &pinned)
        .unwrap();
    assert_eq!(*h.entries[0].payload, Payload::Text("c".repeat(40)));
    let unpinned_bytes: usize = h
        .entries
        .iter()
        .filter(|e| e.id != pinned[0].id)
        .map(|e| e.bytes)
        .sum();
    assert!(unpinned_bytes + pinned[0].bytes <= 200);
    assert_eq!(pinned[0].preview, "a".repeat(40));
}

#[test]
fn newest_first_and_duplicates_move_without_growing() {
    let mut h = History::new(32, 4096, 2048);
    for s in ["a", "b", "a"] {
        h.insert(Payload::Text(s.into())).unwrap();
    }
    assert_eq!(h.entries.len(), 2);
    assert_eq!(*h.entries[0].payload, Payload::Text("a".into()));
    assert_eq!(*h.entries[1].payload, Payload::Text("b".into()));
}

#[test]
fn evicts_oldest_at_count_limit() {
    let mut h = History::new(2, 4096, 2048);
    for s in ["a", "b", "c"] {
        h.insert(Payload::Text(s.into())).unwrap();
    }
    assert_eq!(h.entries.len(), 2);
    assert_eq!(*h.entries[1].payload, Payload::Text("b".into()));
}

#[test]
fn rejects_oversized_items_without_losing_history() {
    let mut h = History::new(32, 1000, 100);
    h.insert(Payload::Text("ok".into())).unwrap();
    assert!(h.insert(Payload::Text("x".repeat(200))).is_err());
    assert_eq!(h.entries.len(), 1);
}

#[test]
fn byte_budget_includes_preview_and_clear_releases_accounting() {
    let mut h = History::new(32, 200, 200);
    for c in ['a', 'b', 'c'] {
        h.insert(Payload::Text(c.to_string().repeat(40))).unwrap();
    }
    assert!(h.bytes <= 200);
    assert_eq!(h.entries.len(), 2);
    h.clear();
    assert_eq!(h.bytes, 0);
    assert!(h.entries.is_empty());
}

#[test]
fn snapshots_remain_stable_when_history_changes() {
    let mut h = History::new(2, 4096, 2048);
    h.insert(Payload::Text("old".into())).unwrap();
    let snapshot = h.entries.clone();
    h.insert(Payload::Text("new".into())).unwrap();
    h.clear();
    assert_eq!(snapshot[0].preview, "old");
}

#[test]
fn page_counts_handle_empty_and_partial_pages() {
    assert_eq!(
        [pages(0), pages(1), pages(8), pages(9), pages(32)],
        [1, 1, 1, 2, 4]
    );
}

#[test]
fn segments_start_at_top_and_exclude_center_and_outside() {
    assert_eq!(hit_segment(0., -110., 4), Some(0));
    assert_eq!(hit_segment(110., 0., 4), Some(1));
    assert_eq!(hit_segment(0., 110., 4), Some(2));
    assert_eq!(hit_segment(-110., 0., 4), Some(3));
    assert_eq!(hit_segment(0., 0., 8), None);
    assert_eq!(hit_segment(250., 0., 8), None);
    assert_eq!(hit_segment(0., -110., 0), None);
    assert_eq!(hit_segment(-110., 0., 1), Some(0));
}

#[test]
fn clamps_to_negative_monitor_work_area() {
    assert_eq!(
        place((-1910, 10), (-1920, 0, 0, 1080), (500, 500)),
        (-1920, 0)
    );
    assert_eq!(
        place((-10, 1070), (-1920, 0, 0, 1080), (500, 500)),
        (-500, 580)
    );
    assert_eq!(
        place((500, 500), (0, 0, 1920, 1080), (400, 400)),
        (300, 300)
    );
}

#[test]
fn compact_wheel_hit_targets_match_280_dip_diameter() {
    assert_eq!(hit_segment(0., -40., 8), Some(0));
    assert_eq!(hit_segment(0., -139., 8), Some(0));
    assert_eq!(hit_segment(0., -145., 8), None);
    assert_eq!(hit_segment(0., -26., 8), None);
}
