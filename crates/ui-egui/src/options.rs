//! The Project Options dialog (Project › Options…): the project's options as a list of rows, each
//! typed as text. Apply checks every row, then sends the changed ones as one `project.setOptions`
//! command, so it is one undo step; a value that doesn't parse or is out of range says why and
//! changes nothing. A new option is a new row in [`ROWS`].

use egui::RichText;
use projectcraft_engine::model::Project;
use serde_json::{Map, Value, json};

use crate::App;
use crate::theme::Tokens;

/// One option the dialog shows: its label, the unit after the field, the `project.setOptions`
/// parameter it sets, its current value as text, and the typed text as a parameter value (or why
/// it can't be one).
pub(crate) struct OptionRow {
    pub label: &'static str,
    pub unit: &'static str,
    pub param: &'static str,
    pub read: fn(&Project) -> String,
    pub parse: fn(&str) -> Result<Value, &'static str>,
}

/// The rows, top to bottom.
pub(crate) const ROWS: [OptionRow; 3] = [
    OptionRow {
        label: "Hours per day",
        unit: "hours",
        param: "hoursPerDay",
        read: |p| hours_text(p.options.minutes_per_day),
        parse: |s| hours(s, 24).ok_or(tl!("Hours per day must be 1 to 24, in whole minutes (such as 7.5).")),
    },
    OptionRow {
        label: "Hours per week",
        unit: "hours",
        param: "hoursPerWeek",
        read: |p| hours_text(p.options.minutes_per_week),
        parse: |s| hours(s, 168).ok_or(tl!("Hours per week must be 1 to 168, in whole minutes (such as 37.5).")),
    },
    OptionRow {
        label: "Days per month",
        unit: "days",
        param: "daysPerMonth",
        read: |p| p.options.days_per_month.to_string(),
        parse: |s| match s.trim().parse::<u32>() {
            Ok(d) if (1..=31).contains(&d) => Ok(json!(d)),
            _ => Err(tl!("Days per month must be a whole number from 1 to 31.")),
        },
    },
];

/// Minutes as hours for the dialog: `8`, `7.5`, `7.25`; minutes that aren't a neat decimal show
/// to two places.
fn hours_text(minutes: u32) -> String {
    let text = format!("{:.2}", f64::from(minutes) / 60.0);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Typed hours from 1 to `max` that are whole minutes, as the command's number.
fn hours(s: &str, max: u32) -> Option<Value> {
    let h = s.trim().parse::<f64>().ok().filter(|h| h.is_finite())?;
    let m = h * 60.0;
    let whole = (m - m.round()).abs() < 1e-6;
    (whole && (60.0..=f64::from(max) * 60.0).contains(&m)).then(|| json!(h))
}

/// The dialog's own state: each row's text as typed, read from the project when it opens.
pub(crate) struct OptionsForm {
    texts: Vec<String>,
}

impl OptionsForm {
    fn read(project: &Project) -> Self {
        OptionsForm { texts: ROWS.iter().map(|r| (r.read)(project)).collect() }
    }

    /// The changed rows as `project.setOptions` parameters; empty when nothing changed. Every row
    /// is checked first, so one bad value sends nothing.
    fn params(&self, project: &Project) -> Result<Map<String, Value>, &'static str> {
        let mut params = Map::new();
        for (row, text) in ROWS.iter().zip(&self.texts) {
            if text.trim() == (row.read)(project) {
                continue;
            }
            params.insert(row.param.to_owned(), (row.parse)(text)?);
        }
        Ok(params)
    }
}

impl App {
    /// Opens the Project Options dialog showing the project's options.
    pub fn open_project_options(&mut self) {
        self.options = Some(OptionsForm::read(&self.session.project));
        self.status.clear();
    }

    /// Project Options: Apply sends the changed options as one command and closes; a refused
    /// value leaves the dialog open with the reason. Cancel or Escape closes and changes nothing.
    pub(crate) fn project_options(&mut self, ctx: &egui::Context) {
        let Some(mut form) = self.options.take() else { return };
        let error = self.status.clone();
        let t = Tokens::get(ctx);
        let (mut apply, mut cancel) = (false, false);
        let modal = egui::Modal::new(egui::Id::new("project_options")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.heading(tl!("Project Options"));
            ui.label(RichText::new(tl!("Converting durations between hours, days, weeks and months")).color(t.text_dim));
            ui.add_space(10.0);
            egui::Grid::new("project_options_rows").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                for (row, text) in ROWS.iter().zip(form.texts.iter_mut()) {
                    let label = ui.label(tl!(row.label));
                    ui.add(egui::TextEdit::singleline(text).desired_width(72.0)).labelled_by(label.id);
                    ui.label(RichText::new(tl!(row.unit)).color(t.text_dim));
                    ui.end_row();
                }
            });
            if !error.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(error.as_str()).color(t.warning));
            }
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                cancel = ui.button(tl!("Cancel")).clicked();
                apply = ui.button(tl!("Apply")).clicked();
            });
        });
        let mut close = cancel || modal.should_close();
        if apply {
            match form.params(&self.session.project) {
                Err(why) => self.status = why.to_owned(),
                Ok(params) if params.is_empty() => close = true,
                Ok(params) => close = self.run("project.setOptions", Value::Object(params)),
            }
        }
        if close {
            self.status.clear();
        } else {
            self.options = Some(form);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hours_show_and_parse_as_whole_minutes() {
        assert_eq!(hours_text(480), "8");
        assert_eq!(hours_text(450), "7.5");
        assert_eq!(hours_text(435), "7.25");
        assert_eq!(hours_text(470), "7.83");
        assert_eq!(hours(" 7.5 ", 24), Some(json!(7.5)));
        for bad in ["", "x", "0", "0.5", "24.5", "7.333", "NaN", "inf", "-8", "1e400"] {
            assert_eq!(hours(bad, 24), None, "{bad}");
        }
    }

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    fn opened() -> Harness<'static, App> {
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 720.0))
            .build_ui_state(|ui, app: &mut App| app.ui(ui), App::new(projectcraft_engine::sample(), None));
        h.run_steps(3);
        h.get_by_label("Project").click();
        h.run_steps(3);
        h.get_by_label("Options…").click();
        h.run_steps(3);
        h
    }

    fn retype(h: &mut Harness<'static, App>, label: &str, text: &str) {
        if let Some(form) = h.state_mut().options.as_mut() {
            let i = ROWS.iter().position(|r| r.label == label).unwrap();
            form.texts[i].clear();
        }
        h.run_steps(1);
        h.get_by_label(label).focus();
        h.run_steps(1);
        h.get_by_label(label).type_text(text);
        h.run_steps(2);
    }

    fn texts(h: &Harness<'static, App>) -> Option<Vec<String>> {
        h.state().options.as_ref().map(|f| f.texts.clone())
    }

    /// Project › Options… shows the project's options; Apply sends the changed ones as one
    /// command, one undo step, and closes. Cancel changes nothing.
    #[test]
    fn apply_sets_the_options_in_one_undo_step() {
        let mut h = opened();
        assert_eq!(texts(&h), Some(vec!["8".to_owned(), "40".to_owned(), "20".to_owned()]));
        retype(&mut h, "Hours per day", "7.5");
        retype(&mut h, "Days per month", "21");
        h.get_by_label("Apply").click();
        h.run_steps(3);
        let o = h.state().session.project.options;
        assert_eq!((o.minutes_per_day, o.minutes_per_week, o.days_per_month), (450, 2400, 21));
        assert!(h.state().options.is_none());
        h.state_mut().session.run("edit.undo", &json!({})).unwrap();
        assert_eq!(h.state().session.project.options, projectcraft_engine::model::Options::default());

        let mut h = opened();
        retype(&mut h, "Hours per week", "37.5");
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        assert!(h.state().options.is_none());
        assert!(!h.state().session.can_undo());
    }

    /// A value that isn't a number, or is out of range, says why, changes nothing (not even a
    /// valid value typed beside it) and keeps the dialog open.
    #[test]
    fn invalid_values_say_why_and_change_nothing() {
        for (label, bad) in [("Hours per day", "25"), ("Hours per week", "lots"), ("Days per month", "0"), ("Hours per day", "7.333")] {
            let mut h = opened();
            let before = h.state().session.project.clone();
            if label != "Hours per day" {
                retype(&mut h, "Hours per day", "6");
            }
            retype(&mut h, label, bad);
            h.get_by_label("Apply").click();
            h.run_steps(3);
            assert_eq!(h.state().session.project, before, "{label} {bad}");
            assert!(h.state().status.starts_with(label), "{}", h.state().status);
            assert!(h.state().options.is_some());
            assert!(!h.state().session.can_undo());
            // The dialog and the status bar both say why.
            assert_eq!(h.query_all_by_label(h.state().status.as_str()).count(), 2);
        }
    }
}
