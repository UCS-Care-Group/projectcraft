//! Quitting the platform's own way.
//!
//! On macOS the app must not leave by eframe closing its window: AppKit's Touch Bar can then
//! crash the process on its next display cycle. `craft-quit` quits through
//! `-[NSApplication terminate:]` instead. It runs only after the app's own close handling, so the
//! unsaved-changes prompt still comes first: while the prompt holds the window open (it cancels
//! the close), nothing quits; Save or Don't Save asks the window to close again, and then it does.
//! Elsewhere `craft-quit` does nothing and eframe closes the window as before.

use eframe::egui::{Context, ViewportCommand, ViewportId};

/// Whether the app is free to quit at the end of this frame: the main window was asked to close
/// (close button, Cmd+Q, the app itself) and the app's close handling did not hold it back.
/// Call it after the app's frame, as [`finish_frame`] does.
pub fn may_quit(ctx: &Context) -> bool {
    ctx.viewport_id() == ViewportId::ROOT
        && ctx.input(|i| i.viewport().close_requested())
        && !ctx.viewport(|v| v.commands.contains(&ViewportCommand::CancelClose))
}

/// The last call of every frame. When the app may quit, quits the platform's own way (macOS);
/// returns whether that took the close over from eframe.
pub fn finish_frame(ctx: &Context) -> bool {
    may_quit(ctx) && craft_quit::on_close_request(ctx)
}

#[cfg(test)]
mod tests {
    use eframe::egui::{self, ViewportEvent};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use projectcraft_engine::Session;
    use projectcraft_engine::model::Project;
    use projectcraft_ui_egui::App;
    use serde_json::json;

    use super::*;

    struct State {
        app: App,
        /// Whether any frame so far ended free to quit (where macOS would quit).
        quit: bool,
    }

    type H = Harness<'static, State>;

    fn harness(app: App) -> H {
        Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(
            // As `Shell`: `logic` (the close check), then `ui`, in one frame.
            |ui, s: &mut State| {
                s.app.logic(ui.ctx());
                s.quit |= may_quit(ui.ctx());
                s.app.ui(ui);
            },
            State { app, quit: false },
        )
    }

    fn app(dirty: bool) -> App {
        let mut app = App::new(Session::new(Project::new("Quit test", projectcraft_engine::time::Date::EPOCH)), None);
        if dirty {
            app.session.run("task.add", &json!({})).unwrap();
            assert!(app.is_dirty());
        }
        app
    }

    /// One frame with a close request: the close button, or Cmd+Q through the app menu.
    fn close(h: &mut H) {
        h.input_mut().viewports.entry(ViewportId::ROOT).or_default().events.push(ViewportEvent::Close);
        h.step();
    }

    /// A few frames as eframe runs them: a `ViewportCommand::Close` the app sent comes back as a
    /// close request in the next frame.
    fn settle(h: &mut H) {
        for _ in 0..4 {
            let asked = h.output().viewport_output.get(&ViewportId::ROOT).is_some_and(|o| o.commands.contains(&ViewportCommand::Close));
            if asked { close(h) } else { h.step() }
        }
    }

    /// Clicks the prompt's button `label`: the one in the row with Don't Save (the toolbar has a
    /// Save of its own).
    fn answer(h: &mut H, label: &str) {
        let row = h.get_by_label("Don't Save").rect().center().y;
        h.get_all_by_label(label).find(|n| (n.rect().center().y - row).abs() < 1.0).unwrap().click();
        h.step();
    }

    #[test]
    fn a_clean_project_quits_at_once() {
        let mut h = harness(app(false));
        h.run_steps(2);
        assert!(!h.state().quit, "no close asked for");
        close(&mut h);
        assert!(h.state().quit);
    }

    #[test]
    fn unsaved_changes_ask_first_and_cancel_keeps_the_app() {
        let mut h = harness(app(true));
        close(&mut h);
        settle(&mut h);
        assert!(!h.state().quit, "the prompt holds the window open");
        assert!(h.query_by_label("Don't Save").is_some(), "the prompt is up");
        answer(&mut h, "Cancel");
        settle(&mut h);
        assert!(!h.state().quit, "Cancel aborts the quit");
        assert!(h.query_by_label("Don't Save").is_none(), "the prompt is gone");
        assert!(h.state().app.is_dirty(), "the changes are still there");
    }

    #[test]
    fn dont_save_quits() {
        let mut h = harness(app(true));
        close(&mut h);
        settle(&mut h);
        assert!(!h.state().quit);
        answer(&mut h, "Don't Save");
        settle(&mut h);
        assert!(h.state().quit);
    }

    #[test]
    fn save_writes_the_file_then_quits() {
        let dir = std::env::temp_dir().join(format!("projectcraft-quit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("q.projectcraft");
        let mut a = app(false);
        a.file_picked(projectcraft_ui_egui::FileRequest::SaveAs, Some(file.clone()));
        a.session.run("task.add", &json!({})).unwrap();
        assert!(a.is_dirty());
        let mut h = harness(a);
        close(&mut h);
        settle(&mut h);
        assert!(!h.state().quit);
        answer(&mut h, "Save");
        settle(&mut h);
        assert!(h.state().quit);
        assert!(!h.state().app.is_dirty());
        assert!(file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_quits_without_a_close_request() {
        let mut h = harness(app(true));
        h.run_steps(3);
        assert!(!h.state().quit);
    }

    /// A close while minimized: eframe runs only `logic`, never `ui`. Returns whether the app
    /// may quit and the root window's commands.
    fn minimized_close(mut a: App) -> (bool, Vec<ViewportCommand>) {
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        let root = input.viewports.entry(ViewportId::ROOT).or_default();
        root.minimized = Some(true);
        root.events.push(ViewportEvent::Close);
        let mut quit = false;
        let mut out = ctx.run_logic(&input, |ctx| {
            a.logic(ctx);
            quit = may_quit(ctx);
        });
        (quit, out.viewport_commands.remove(&ViewportId::ROOT).unwrap_or_default())
    }

    #[test]
    fn minimized_with_unsaved_changes_comes_back_to_ask() {
        let (quit, commands) = minimized_close(app(true));
        assert!(!quit);
        assert!(commands.contains(&ViewportCommand::CancelClose));
        assert!(commands.contains(&ViewportCommand::Minimized(false)), "the prompt needs the window shown");
    }

    #[test]
    fn minimized_and_clean_quits() {
        let (quit, commands) = minimized_close(app(false));
        assert!(quit);
        assert!(!commands.contains(&ViewportCommand::CancelClose));
    }
}
