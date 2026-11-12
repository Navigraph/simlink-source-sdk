//! Rust names from manifest names.

/// `telemetry.ownship` → `telemetry`, `ownship`; `sourceId` → `source`, `Id`.
fn words(name: &str) -> Vec<&str> {
    let mut words = Vec::new();
    for part in name.split(|c: char| !c.is_ascii_alphanumeric()) {
        let mut start = 0;
        for (i, c) in part.char_indices().skip(1) {
            if c.is_ascii_uppercase() {
                words.push(&part[start..i]);
                start = i;
            }
        }
        words.push(&part[start..]);
    }
    words.retain(|word| !word.is_empty());
    words
}

/// `telemetry.ownship` → `TelemetryOwnship`.
pub fn pascal(name: &str) -> String {
    words(name)
        .into_iter()
        .map(|word| word[..1].to_ascii_uppercase() + &word[1..])
        .collect()
}

/// `telemetry.ownship` → `telemetry_ownship`; `sourceId` → `source_id`.
pub fn snake(name: &str) -> String {
    words(name)
        .into_iter()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("_")
}

/// Joins the non-empty parts as sentences, so a channel without a description still reads well.
pub fn sentences(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| part.trim().trim_end_matches('.'))
        .filter(|part| !part.is_empty())
        .map(|part| format!("{part}."))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_channel_and_field_names() {
        assert_eq!(pascal("telemetry.ownship"), "TelemetryOwnship");
        assert_eq!(pascal("source_id_in_use"), "SourceIdInUse");
        assert_eq!(snake("telemetry.ownship"), "telemetry_ownship");
        assert_eq!(snake("retryAfterMs"), "retry_after_ms");
    }

    #[test]
    fn joins_sentences_without_empty_parts() {
        assert_eq!(
            sentences(&["Position.", "", " Sent at 10 Hz "]),
            "Position. Sent at 10 Hz."
        );
        assert_eq!(sentences(&["", ""]), "");
    }
}
