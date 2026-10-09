use super::*;

#[test]
fn projection_prefix_and_exclusive_cursor_pages_match_ordered_records() {
    let journal = ephemeral_journal();
    let keys = [
        "a",
        "item:",
        "item:01",
        "item:02",
        "item:02-extra",
        "item:10",
        "items:01",
        "z",
        "é:01",
        "é:02",
        "🦀:01",
    ];
    journal
        .apply_all(&["rows", "rows-extra", "other"].map(|projection| {
            ProjectionBatch {
                projection: projection.into(),
                expected_position: 0,
                through_sequence: 1,
                mutations: keys
                    .iter()
                    .map(|key| ProjectionMutation::Upsert {
                        key: (*key).into(),
                        value: json!({"projection": projection, "key": key}),
                    })
                    .collect(),
            }
        }))
        .expect("seed namespaces");
    for projection in ["rows", "rows-extra", "other", "missing"] {
        for prefix in ["", "item:", "item:02", "items:", "missing:", "é:", "🦀:"] {
            for cursor in [
                None,
                Some(""),
                Some("a"),
                Some("item:"),
                Some("item:02"),
                Some("item:03"),
                Some("z"),
                Some("é:01"),
                Some("🦀:99"),
            ] {
                for limit in [0, 1, 2, 20] {
                    let expected = keys
                        .iter()
                        .filter(|key| {
                            projection != "missing"
                                && key.starts_with(prefix)
                                && cursor.is_none_or(|cursor| **key > cursor)
                        })
                        .take(limit)
                        .map(|key| {
                            (
                                (*key).to_owned(),
                                json!({"projection": projection, "key": key}),
                            )
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(
                        journal
                            .list_after(projection, prefix, cursor, limit)
                            .expect("page"),
                        expected,
                        "projection={projection}, prefix={prefix}, cursor={cursor:?}, limit={limit}"
                    );
                    if cursor.is_none() {
                        assert_eq!(
                            journal
                                .list(projection, prefix, limit)
                                .expect("prefix page"),
                            expected
                        );
                    }
                }
            }
        }
    }
    journal.reset("rows").expect("reset one namespace");
    assert_eq!(journal.position("rows").expect("reset position"), 0);
    assert!(journal.list("rows", "", 20).expect("reset rows").is_empty());
    assert_eq!(
        journal
            .list("rows-extra", "", 20)
            .expect("neighbor rows")
            .len(),
        keys.len()
    );
    for (projection, prefix, cursor) in [
        ("", "", None),
        ("rows\0bad", "", None),
        ("rows", "bad\0prefix", None),
        ("rows", "", Some("bad\0cursor")),
    ] {
        assert!(matches!(
            journal.list_after(projection, prefix, cursor, 0),
            Err(StoreError::Adapter(_))
        ));
    }
}
