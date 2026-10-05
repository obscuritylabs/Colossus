use super::*;
fn recurrence(zone: &str, time: &str, days: &[u8]) -> WorkflowCalendar {
    WorkflowCalendar {
        timezone: zone.into(),
        time: time.into(),
        weekdays: days.into(),
    }
}
fn instant(value: &str) -> OffsetDateTime {
    parse_schedule_time(value, "test").unwrap()
}
fn after(calendar: &WorkflowCalendar, value: &str) -> String {
    format_schedule_time(next(calendar, instant(value)).unwrap()).unwrap()
}
#[test]
fn daily_preserves_wall_time_across_both_new_york_clock_changes() {
    let calendar = recurrence("America/New_York", "09:00", &[]);
    assert_eq!(
        after(&calendar, "2026-03-07T14:00:00Z"),
        "2026-03-08T13:00:00Z"
    );
    assert_eq!(
        after(&calendar, "2026-10-31T13:00:00Z"),
        "2026-11-01T14:00:00Z"
    );
}
#[test]
fn weekly_selected_days_use_local_weekdays_and_dst_offsets() {
    let calendar = recurrence("America/New_York", "09:00", &[1, 5]);
    assert_eq!(
        after(&calendar, "2026-03-06T14:00:00Z"),
        "2026-03-09T13:00:00Z"
    );
    assert_eq!(
        after(&calendar, "2026-03-09T13:00:00Z"),
        "2026-03-13T13:00:00Z"
    );
}
#[test]
fn nonexistent_times_are_skipped_and_repeated_times_fire_once() {
    let gap = recurrence("America/New_York", "02:30", &[]);
    assert_eq!(after(&gap, "2026-03-07T07:30:00Z"), "2026-03-09T06:30:00Z");
    let fold = recurrence("America/New_York", "01:30", &[]);
    assert_eq!(after(&fold, "2026-10-31T05:30:00Z"), "2026-11-01T05:30:00Z");
    assert_eq!(after(&fold, "2026-11-01T05:30:00Z"), "2026-11-02T06:30:00Z");
    assert!(validate(&fold, instant("2026-11-01T06:30:00Z")).is_err());
}
#[test]
fn half_hour_transitions_and_skipped_civil_days_are_supported() {
    let half_hour = recurrence("Australia/Lord_Howe", "02:15", &[]);
    assert_eq!(
        after(&half_hour, "2026-10-02T15:45:00Z"),
        "2026-10-04T15:15:00Z"
    );
    let skipped_day = recurrence("Pacific/Apia", "09:00", &[]);
    assert_eq!(
        after(&skipped_day, "2011-12-29T19:00:00Z"),
        "2011-12-30T19:00:00Z"
    );
}
#[test]
fn invalid_calendar_and_mismatched_first_occurrences_fail_closed() {
    for calendar in [
        recurrence("Unknown/Zone", "09:00", &[]),
        recurrence("UTC", "24:00", &[]),
        recurrence("UTC", "9:00", &[]),
        recurrence("UTC", "09:00", &[1, 1]),
        recurrence("UTC", "09:00", &[0]),
    ] {
        assert!(validate(&calendar, instant("2026-03-09T09:00:00Z")).is_err());
    }
    assert!(
        validate(
            &recurrence("UTC", "09:00", &[1]),
            instant("2026-03-10T09:00:00Z")
        )
        .is_err()
    );
    assert!(
        validate(
            &recurrence("UTC", "09:00", &[]),
            instant("2026-03-09T09:00:01Z")
        )
        .is_err()
    );
}
