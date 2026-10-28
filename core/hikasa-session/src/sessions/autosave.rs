//! Auto-save groups (D-83): an auto-saved session falls in Today, Yesterday, 2–6 days ago or More than 6 days
//! ago, by its `saved_at` date against the injected local date (future dates count as Today). The rotation
//! itself is [`super::SessionStore::autosave`] and [`super::SessionStore::prune_autosaved`].

use jiff::civil::Date;
use serde::Serialize;

/// The group of an auto-saved session (SES-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoGroup {
    /// Saved today (or in the future).
    Today,
    /// Saved yesterday.
    Yesterday,
    /// Saved 2 to 6 days ago.
    #[serde(rename = "days_2_to_6")]
    Days2To6,
    /// Saved more than 6 days ago.
    Older,
}

impl AutoGroup {
    /// The JSON name (`today`, `yesterday`, `days_2_to_6`, `older`).
    pub const fn as_str(self) -> &'static str {
        match self {
            AutoGroup::Today => "today",
            AutoGroup::Yesterday => "yesterday",
            AutoGroup::Days2To6 => "days_2_to_6",
            AutoGroup::Older => "older",
        }
    }

    /// The caption of the tree node.
    pub const fn caption(self) -> &'static str {
        match self {
            AutoGroup::Today => "Today",
            AutoGroup::Yesterday => "Yesterday",
            AutoGroup::Days2To6 => "2–6 days ago",
            AutoGroup::Older => "More than 6 days ago",
        }
    }
}

/// The group of a session saved on `saved`, seen on `today`.
pub fn auto_group(saved: Date, today: Date) -> AutoGroup {
    let days = saved.until(today).map_or(0, |span| span.get_days());
    match days {
        i32::MIN..=0 => AutoGroup::Today,
        1 => AutoGroup::Yesterday,
        2..=6 => AutoGroup::Days2To6,
        _ => AutoGroup::Older,
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    #[test]
    fn group_boundaries() {
        let today = date(2026, 10, 28);
        let rows = [
            (date(2026, 10, 30), AutoGroup::Today),
            (date(2026, 10, 28), AutoGroup::Today),
            (date(2026, 10, 27), AutoGroup::Yesterday),
            (date(2026, 10, 26), AutoGroup::Days2To6),
            (date(2026, 10, 22), AutoGroup::Days2To6),
            (date(2026, 10, 21), AutoGroup::Older),
            (date(2025, 10, 28), AutoGroup::Older),
            // Across a month and a year end.
            (date(2026, 9, 30), AutoGroup::Older),
        ];
        for (saved, group) in rows {
            assert_eq!(auto_group(saved, today), group, "{saved}");
        }
        assert_eq!(
            auto_group(date(2025, 12, 31), date(2026, 1, 1)),
            AutoGroup::Yesterday
        );
    }
}
