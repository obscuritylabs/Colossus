//! Closed failure evidence: fixed categories/OSStatus integers, never raw output.
const CATEGORIES: [&str; 5] = [
    "fixture_admission_or_import_rejected",
    "fixture_import_unknown",
    "fixture_import_retirement_unknown",
    "fixture_native_retirement_unknown",
    "fixture_tls_drainage_unknown_store_retained",
];
pub fn capture(bytes: &[u8]) -> Result<(Vec<serde_json::Value>, Option<&'static str>), ()> {
    let text = std::str::from_utf8(bytes).map_err(|_| ())?;
    let mut traces = Vec::new();
    let mut category = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("colossus_native_custody_trace: ") {
            if category.is_some() || traces.len() >= 2 {
                return Err(());
            }
            let trace: serde_json::Value = serde_json::from_str(value).map_err(|_| ())?;
            let object = trace.as_object().ok_or(())?;
            if object.len() != 4
                || !["operation", "phase", "check", "os_status"]
                    .iter()
                    .all(|key| object.contains_key(*key))
                || !matches!(trace["operation"].as_str(), Some("prepare" | "retire"))
                || trace["phase"].as_u64().is_none_or(|value| value > 6)
                || trace["check"].as_u64().is_none_or(|value| value > 25)
                || trace["os_status"]
                    .as_i64()
                    .is_none_or(|value| i32::try_from(value).is_err())
                || traces
                    .iter()
                    .any(|prior: &serde_json::Value| prior["operation"] == trace["operation"])
            {
                return Err(());
            }
            traces.push(trace);
        } else if let Some(value) = line.strip_prefix("colossus_native_custody: ") {
            if category.is_some() {
                return Err(());
            }
            category = Some(*CATEGORIES.iter().find(|known| **known == value).ok_or(())?);
        } else {
            return Err(());
        }
    }
    Ok((traces, category))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captures_only_finite_native_categories_and_integer_statuses() {
        let bytes = b"colossus_native_custody_trace: {\"operation\":\"prepare\",\"phase\":2,\"check\":6,\"os_status\":-25295}\ncolossus_native_custody: fixture_import_unknown\n";
        let (traces, category) = capture(bytes).expect("closed trace");
        assert_eq!(traces.len(), 1);
        assert_eq!(category, Some("fixture_import_unknown"));
        for bad in [
            "secret",
            "colossus_native_custody_trace: {\"operation\":\"prepare\",\"phase\":7,\"check\":6,\"os_status\":0}",
            "colossus_native_custody_trace: {\"operation\":\"prepare\",\"phase\":2,\"check\":6,\"os_status\":0,\"path\":\"private\"}",
        ] {
            assert!(capture(bad.as_bytes()).is_err());
        }
    }
}
