//! Bounded wall-clock recurrence. Gaps are skipped; folds fire at the earlier instant once.
use super::*;
use colossus_contracts::WorkflowCalendar;
use jiff::{
    Timestamp,
    civil::Date,
    tz::{AmbiguousOffset, TimeZone},
};

fn invalid() -> WorkflowError {
    WorkflowError::InvalidDefinition("invalid calendar recurrence or occurrence".into())
}

fn parts(calendar: &WorkflowCalendar) -> Result<(TimeZone, i8, i8), WorkflowError> {
    if calendar.timezone.is_empty()
        || calendar.timezone.len() > 128
        || calendar.weekdays.len() > 7
        || calendar.weekdays.iter().any(|day| !(1..=7).contains(day))
        || calendar.weekdays.windows(2).any(|days| days[0] >= days[1])
        || calendar.time.len() != 5
        || calendar.time.as_bytes()[2] != b':'
        || !calendar
            .time
            .bytes()
            .enumerate()
            .all(|(i, b)| i == 2 || b.is_ascii_digit())
    {
        return Err(invalid());
    }
    let hour: i8 = calendar.time[..2].parse().map_err(|_| invalid())?;
    let minute: i8 = calendar.time[3..].parse().map_err(|_| invalid())?;
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) {
        return Err(invalid());
    }
    let zone = TimeZone::get(&calendar.timezone).map_err(|_| invalid())?;
    if zone.iana_name().is_none() {
        return Err(invalid());
    }
    Ok((zone, hour, minute))
}

fn candidate(
    calendar: &WorkflowCalendar,
    zone: &TimeZone,
    date: Date,
    hour: i8,
    minute: i8,
) -> Result<Option<Timestamp>, WorkflowError> {
    let day = u8::try_from(date.weekday().to_monday_one_offset()).map_err(|_| invalid())?;
    if !calendar.weekdays.is_empty() && !calendar.weekdays.contains(&day) {
        return Ok(None);
    }
    let ambiguous = zone.to_ambiguous_zoned(date.at(hour, minute, 0, 0));
    if matches!(ambiguous.offset(), AmbiguousOffset::Gap { .. }) {
        return Ok(None);
    }
    Ok(Some(
        ambiguous.earlier().map_err(|_| invalid())?.timestamp(),
    ))
}

/// Validate the exact reviewed first instant against the local recurrence.
pub(super) fn validate(
    calendar: &WorkflowCalendar,
    first: OffsetDateTime,
) -> Result<(), WorkflowError> {
    let (zone, hour, minute) = parts(calendar)?;
    if first.nanosecond() != 0 {
        return Err(invalid());
    }
    let instant = Timestamp::from_second(first.unix_timestamp()).map_err(|_| invalid())?;
    let date = instant.to_zoned(zone.clone()).date();
    if candidate(calendar, &zone, date, hour, minute)? != Some(instant) {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn next(
    calendar: &WorkflowCalendar,
    after: OffsetDateTime,
) -> Result<OffsetDateTime, WorkflowError> {
    let (zone, hour, minute) = parts(calendar)?;
    let instant = Timestamp::from_second(after.unix_timestamp()).map_err(|_| invalid())?;
    let mut date = instant.to_zoned(zone.clone()).date();
    // Includes a skipped civil day and a weekly DST gap without an unbounded search.
    for _ in 0..16 {
        if let Some(value) = candidate(calendar, &zone, date, hour, minute)?
            && value > instant
        {
            return OffsetDateTime::from_unix_timestamp(value.as_second()).map_err(|_| invalid());
        }
        date = date.tomorrow().map_err(|_| invalid())?;
    }
    Err(invalid())
}

pub(super) fn due(
    schedule: &WorkflowSchedule,
    first: OffsetDateTime,
    now: OffsetDateTime,
) -> Result<(u64, OffsetDateTime, OffsetDateTime), WorkflowError> {
    let Some(calendar) = &schedule.calendar else {
        if !(MIN_SCHEDULE_CADENCE_SECONDS..=MAX_SCHEDULE_CADENCE_SECONDS)
            .contains(&schedule.cadence_seconds)
        {
            return Err(invalid());
        }
        let cadence = i64::try_from(schedule.cadence_seconds).map_err(|_| invalid())?;
        let count =
            u64::try_from((now - first).whole_seconds() / cadence + 1).map_err(|_| invalid())?;
        return Ok((
            count,
            add_schedule_occurrences(first, schedule.cadence_seconds, count - 1)?,
            add_schedule_occurrences(first, schedule.cadence_seconds, count)?,
        ));
    };
    validate(calendar, first)?;
    let mut latest = first;
    for count in 1..=10_000 {
        let future = next(calendar, latest)?;
        if future > now {
            return Ok((count, latest, future));
        }
        latest = future;
    }
    Err(WorkflowError::InvalidTransition(
        "calendar catch-up exceeds 10,000 occurrences; create a new schedule".into(),
    ))
}

#[cfg(test)]
mod tests;
