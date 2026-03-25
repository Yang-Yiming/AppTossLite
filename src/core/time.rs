use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::core::error::{Result, TossError};

pub fn now_rfc3339() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|err| TossError::Config(format!("failed to format timestamp: {err}")))
}

pub fn format_last_tossed(raw: Option<&str>) -> String {
    match raw {
        None => "<never>".to_string(),
        Some(raw) => match OffsetDateTime::parse(raw, &Rfc3339) {
            Ok(timestamp) => {
                let days = (OffsetDateTime::now_utc() - timestamp).whole_days().max(0);
                format!("{raw} ({days} days ago)")
            }
            Err(_) => raw.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_missing_last_tossed() {
        assert_eq!(format_last_tossed(None), "<never>");
    }

    #[test]
    fn leaves_invalid_timestamp_unchanged() {
        assert_eq!(
            format_last_tossed(Some("not-a-timestamp")),
            "not-a-timestamp"
        );
    }

    #[test]
    fn formats_valid_timestamp_with_days_ago_suffix() {
        let formatted = format_last_tossed(Some("1970-01-01T00:00:00Z"));

        assert!(formatted.contains("1970-01-01T00:00:00Z"));
        assert!(formatted.contains("days ago"));
    }
}
