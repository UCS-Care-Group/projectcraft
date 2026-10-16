//! What the task sheet shows: its columns, kept with the project as Microsoft Project keeps a
//! table's columns in the file. ID and Task Name always show; the rest can be hidden.

use serde::{Deserialize, Deserializer, Serialize};

use crate::ModelError;

/// A task sheet column that can be hidden. ID and Task Name always show, so they are not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Column {
    Duration,
    Work,
    Start,
    Finish,
    Predecessors,
    ResourceNames,
    Cost,
    FixedCost,
    Deadline,
    Type,
    EffortDriven,
}

impl Column {
    /// Every column, left to right as a new project shows them.
    pub const ALL: [Column; 11] = [
        Column::Duration,
        Column::Work,
        Column::Start,
        Column::Finish,
        Column::Predecessors,
        Column::ResourceNames,
        Column::Cost,
        Column::FixedCost,
        Column::Deadline,
        Column::Type,
        Column::EffortDriven,
    ];

    /// Its heading.
    pub fn name(self) -> &'static str {
        match self {
            Column::Duration => "Duration",
            Column::Work => "Work",
            Column::Start => "Start",
            Column::Finish => "Finish",
            Column::Predecessors => "Predecessors",
            Column::ResourceNames => "Resource Names",
            Column::Cost => "Cost",
            Column::FixedCost => "Fixed Cost",
            Column::Deadline => "Deadline",
            Column::Type => "Type",
            Column::EffortDriven => "Effort Driven",
        }
    }

    /// From its heading, any case.
    pub fn from_name(text: &str) -> Option<Column> {
        let t = text.trim();
        Column::ALL.into_iter().find(|c| c.name().eq_ignore_ascii_case(t))
    }
}

/// The task sheet's columns after ID and Task Name, left to right; each at most once. A file
/// without them shows every column; a column name this version doesn't know is dropped on load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Columns(Vec<Column>);

impl Default for Columns {
    fn default() -> Self {
        Columns(Column::ALL.to_vec())
    }
}

impl<'de> Deserialize<'de> for Columns {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // Any item, so a column from a newer version can't stop the file opening.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Item {
            Known(Column),
            Other(serde::de::IgnoredAny),
        }
        let items: Vec<Item> = Vec::deserialize(d)?;
        Ok(Columns::new(items.into_iter().filter_map(|i| match i {
            Item::Known(c) => Some(c),
            Item::Other(_) => None,
        })))
    }
}

impl Columns {
    /// These columns, left to right; a repeat counts once.
    pub fn new(list: impl IntoIterator<Item = Column>) -> Self {
        let mut shown = Vec::new();
        for c in list {
            if !shown.contains(&c) {
                shown.push(c);
            }
        }
        Columns(shown)
    }

    /// From headings (`Duration`, `Resource Names`), any case. `ID` and `Task Name` always show,
    /// so they may be listed and are passed over; anything else unknown is refused.
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<Self, ModelError> {
        let mut list = Vec::new();
        for n in names {
            if ["id", "task name", "name"].contains(&n.trim().to_lowercase().as_str()) {
                continue;
            }
            let c = Column::from_name(n).ok_or_else(|| {
                let n: String = n.chars().take(30).collect();
                let all: Vec<&str> = Column::ALL.iter().map(|c| c.name()).collect();
                ModelError::Invalid(format!("`{n}` is not a column; expected one of {}", all.join(", ")))
            })?;
            list.push(c);
        }
        Ok(Columns::new(list))
    }

    pub fn shown(&self) -> &[Column] {
        &self.0
    }

    pub fn contains(&self, c: Column) -> bool {
        self.0.contains(&c)
    }

    pub fn is_default(&self) -> bool {
        *self == Columns::default()
    }

    /// These columns with `c` shown or hidden. A column shown again goes back after the shown
    /// columns that come before it in [`Column::ALL`], so the usual order is kept.
    pub fn with(&self, c: Column, show: bool) -> Columns {
        let mut list = self.0.clone();
        list.retain(|x| *x != c);
        if show {
            let before: Vec<Column> = Column::ALL.iter().copied().take_while(|x| *x != c).collect();
            let at = list.iter().rposition(|x| before.contains(x)).map_or(0, |i| i.saturating_add(1));
            list.insert(at.min(list.len()), c);
        }
        Columns(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_unknown_ones_are_refused() {
        for c in Column::ALL {
            assert_eq!(Column::from_name(&c.name().to_uppercase()), Some(c));
        }
        let cols = Columns::from_names(["ID", "Task Name", "resource names", "Duration", "Duration"]).unwrap();
        assert_eq!(cols.shown(), &[Column::ResourceNames, Column::Duration]);
        let e = Columns::from_names(["Colour"]).unwrap_err().to_string();
        assert!(e.contains("not a column") && e.contains("Effort Driven"), "{e}");
    }

    #[test]
    fn a_column_shown_again_goes_back_in_its_place() {
        let all = Columns::default();
        let hidden = all.with(Column::Start, false).with(Column::Duration, false);
        assert!(!hidden.contains(Column::Start) && !hidden.contains(Column::Duration));
        assert_eq!(hidden.with(Column::Start, true).with(Column::Duration, true), all);
        let none = Columns::new([]);
        assert_eq!(none.with(Column::Cost, true).shown(), &[Column::Cost]);
    }
}
