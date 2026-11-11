//! Autosave, crash recovery and the quit question (ADR-34).
//!
//! The picture counts as unsaved when the engine revision differs from the one at the last
//! Save or Load. While it is unsaved, a recovery project is written after 30 s without a
//! change (or after 120 s of steady changes) and when the program quits.

use std::path::PathBuf;

use dpx_ui::{ChoiceRequester, ChoiceResponse, ReqEvent};
use eframe::egui;

use crate::recovery::{self, Meta, Store};
use crate::{PainterApp, View, files, note_requester};

/// Seconds without a change before the recovery project is written.
pub const IDLE_SECS: f64 = 30.0;
/// Seconds of steady changes after which it is written anyway.
pub const MAX_SECS: f64 = 120.0;

const RECOVER_TITLE: &str = "Recover Picture";
const QUIT_TITLE: &str = "Quit";

/// What the open choice requester is asking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pending {
    /// Recover / Discard at start.
    Recover,
    /// Yes / No: quit with unsaved changes.
    Quit,
}

pub struct State {
    /// Where the recovery project goes; `None` = no autosave (snapshots, most tests).
    pub store: Option<Store>,
    /// Engine revision of the last Save or Load; `None` = the picture was recovered and is
    /// unsaved whatever its revision.
    pub saved_revision: Option<u64>,
    /// Last revision looked at, and when it changed (UI clock, seconds).
    seen_revision: u64,
    changed_at: f64,
    /// When the oldest change not yet written happened.
    first_change: Option<f64>,
    /// Revision the recovery project holds.
    written_revision: Option<u64>,
    pub pending: Option<Pending>,
    /// The user said Yes to "Quit without saving?"; the window may close.
    pub quit_confirmed: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            store: None,
            saved_revision: Some(0),
            seen_revision: 0,
            changed_at: 0.0,
            first_change: None,
            written_revision: None,
            pending: None,
            quit_confirmed: false,
        }
    }
}

/// The "Recover Picture" requester for the picture `name`, autosaved at `when`.
pub fn recover_requester(name: &str, when: String) -> ChoiceRequester {
    ChoiceRequester::new(
        RECOVER_TITLE,
        &[
            "Unsaved changes were found.".to_string(),
            format!("Picture: \"{name}\""),
            format!("Autosaved: {when}"),
        ],
        ["Recover", "Discard"],
        0,
        1,
    )
}

impl PainterApp {
    /// The picture has changes that are in no saved file.
    pub fn unsaved(&self) -> bool {
        self.autosave.saved_revision != Some(self.engine.document().revision())
    }

    /// A Save or Load happened: the picture is clean, the recovery project is not needed.
    pub fn mark_clean(&mut self) {
        let rev = self.engine.document().revision();
        let a = &mut self.autosave;
        a.saved_revision = Some(rev);
        a.seen_revision = rev;
        a.first_change = None;
        a.written_revision = None;
        if a.pending.is_none()
            && let Some(store) = &a.store
        {
            store.discard();
        }
    }

    /// Writes the recovery project now. Errors go to stderr; the picture is not harmed.
    pub fn write_recovery(&mut self) {
        let Some(store) = &self.autosave.store else {
            return;
        };
        let path = store.project_path();
        let result = files::encode_document(self.engine.document(), &self.extras, &path)
            .map_err(|e| format!("recovery: {e}"))
            .and_then(|(bytes, _)| {
                let meta = Meta {
                    picture: self.picture_path.clone(),
                    saved_unix: recovery::unix_now(),
                };
                store.write(&bytes, &meta)
            });
        match result {
            Ok(()) => {
                self.autosave.written_revision = Some(self.engine.document().revision());
                self.autosave.first_change = None;
            }
            Err(e) => eprintln!("{e}"),
        }
    }

    /// Called every frame with the UI clock in seconds: writes the recovery project when it
    /// is due. Returns the seconds until it is due, if it is waiting.
    pub fn autosave_tick(&mut self, now: f64) -> Option<f64> {
        let rev = self.engine.document().revision();
        let a = &mut self.autosave;
        if rev != a.seen_revision {
            a.seen_revision = rev;
            a.changed_at = now;
            a.first_change.get_or_insert(now);
        }
        if a.store.is_none()
            || a.pending == Some(Pending::Recover)
            || !self.unsaved()
            || self.autosave.written_revision == Some(rev)
        {
            return None;
        }
        let a = &mut self.autosave;
        let first = *a.first_change.get_or_insert(now);
        let wait = (a.changed_at + IDLE_SECS - now).min(first + MAX_SECS - now);
        if wait <= 0.0 {
            self.write_recovery();
            None
        } else {
            Some(wait)
        }
    }

    /// Hands the recovery folder to the app and, if a recovery project is waiting from an
    /// earlier run, asks what to do with it (or discards it with a note when it is damaged).
    pub fn attach_recovery(&mut self, store: Store) {
        let waiting = store.exists();
        self.autosave.store = Some(store.clone());
        if !waiting {
            return;
        }
        if let Err(e) = files::load(&store.project_path()) {
            eprintln!("recovery file discarded: {e}");
            store.discard();
            let reason = e.lines().next().unwrap_or("").to_string();
            self.ui.note_requester = note_requester(
                "Recovery discarded",
                &[format!("the recovery file is damaged ({reason})")],
            );
            return;
        }
        let meta = store.meta();
        let when = if meta.saved_unix == 0 {
            "an unknown time".to_string()
        } else {
            recovery::format_utc(meta.saved_unix)
        };
        self.ui.choice_requester = Some(recover_requester(&meta.name(), when));
        self.autosave.pending = Some(Pending::Recover);
    }

    /// Picture ▸ Quit and the window's close button.
    pub fn request_quit(&mut self, ctx: &egui::Context) {
        if self.autosave.pending == Some(Pending::Recover) || self.autosave.quit_confirmed {
            return ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.unsaved() {
            self.finish_stroke();
            self.ui.choice_requester = Some(ChoiceRequester::new(
                QUIT_TITLE,
                &["Quit without saving?".to_string()],
                ["Yes", "No"],
                1,
                1,
            ));
            self.autosave.pending = Some(Pending::Quit);
            self.dirty = true;
        } else {
            self.finish_session();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// The program is quitting: keep the unsaved picture in the recovery slot, or empty the
    /// slot when there is nothing to keep. A recovery project still waiting for the owner's
    /// answer stays untouched.
    pub fn finish_session(&mut self) {
        if self.autosave.pending == Some(Pending::Recover) {
            return;
        }
        if self.unsaved() {
            self.write_recovery();
        } else if let Some(store) = &self.autosave.store {
            store.discard();
        }
    }

    /// The window was asked to close (close button, Cmd+Q).
    pub fn close_requested(&mut self, ctx: &egui::Context) {
        if self.autosave.quit_confirmed || self.autosave.pending == Some(Pending::Recover) {
            self.finish_session();
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        self.request_quit(ctx);
    }

    /// Pointer and keys while the choice requester is shown (it is modal).
    pub fn choice_input(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
        ppp: f32,
        scale: f32,
        view: View,
    ) {
        let Some(req) = &self.ui.choice_requester else {
            return;
        };
        let ui_screen = view.ui_screen();
        let at = req.origin(ui_screen.width, ui_screen.height);
        let (events, _) = self.modal_events(ctx, origin, ppp, scale, view, at);
        for event in events {
            if self.ui.choice_requester.is_none() {
                break;
            }
            self.choice_event(event, ctx);
        }
    }

    /// Feeds one event to the choice requester and acts on the answer.
    pub fn choice_event(&mut self, event: ReqEvent, ctx: &egui::Context) {
        let Some(req) = &mut self.ui.choice_requester else {
            return;
        };
        self.dirty = true;
        let ChoiceResponse::Pick(button) = req.handle(event) else {
            return;
        };
        self.ui.choice_requester = None;
        match self.autosave.pending.take() {
            Some(Pending::Recover) if button == 0 => self.recover(),
            Some(Pending::Recover) => {
                if let Some(store) = &self.autosave.store {
                    store.discard();
                }
            }
            Some(Pending::Quit) if button == 0 => {
                self.autosave.quit_confirmed = true;
                self.finish_session();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => {}
        }
    }

    /// Opens the recovery project like Load; the picture is unsaved and keeps its old name.
    fn recover(&mut self) {
        let Some(store) = self.autosave.store.clone() else {
            return;
        };
        let loaded = files::load(&store.project_path());
        let loaded = match loaded {
            Ok(l) => l,
            Err(e) => {
                eprintln!("recovery file discarded: {e}");
                store.discard();
                let reason = e.lines().next().unwrap_or("").to_string();
                self.ui.note_requester = note_requester(
                    "Recovery discarded",
                    &[format!("the recovery file is damaged ({reason})")],
                );
                return;
            }
        };
        let original: Option<PathBuf> = store.meta().picture;
        let current = self.engine.document().target();
        let (doc, extras, notes) = files::open_picture(loaded.picture, current);
        let (doc, grown) = files::page_at_least_screen(doc);
        let notes: Vec<String> = loaded.notes.into_iter().chain(notes).chain(grown).collect();
        let placeholder = original.clone().unwrap_or_else(|| store.project_path());
        self.set_picture(doc, extras, placeholder);
        self.picture_path = original;
        if self.picture_path.is_none() {
            self.last_drawer = None;
        }
        // Not saved anywhere yet: keep it in the recovery slot until the owner saves it.
        self.autosave.saved_revision = None;
        self.write_recovery();
        self.ui.note_requester = note_requester("Picture recovered", &notes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::Command;
    use dpx_ui::ReqKey;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dpx-autosave-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn paint(app: &mut PainterApp, x: i32, colour: u8) {
        app.engine
            .execute(Command::Stroke {
                brush: vec![(0, 0)],
                color: colour,
                points: vec![(x, 3)],
                continuous: false,
                custom: None,
                mode: Default::default(),
                range: None,
                stencil: None,
                right: false,
                fx: Default::default(),
            })
            .unwrap();
    }

    fn app_with_store(dir: &std::path::Path) -> PainterApp {
        let mut app = PainterApp::new();
        app.attach_recovery(Store::at(dir));
        app
    }

    #[test]
    fn autosave_waits_for_idle_time_after_a_change() {
        let dir = temp("idle");
        let mut app = app_with_store(&dir);
        let store = Store::at(&dir);
        assert!(!app.unsaved());
        assert_eq!(app.autosave_tick(5.0), None);
        paint(&mut app, 1, 5);
        assert!(app.unsaved());
        let wait = app.autosave_tick(10.0).expect("waiting");
        assert!((wait - IDLE_SECS).abs() < 1e-9);
        assert!(!store.exists());
        // Another change restarts the idle time.
        paint(&mut app, 2, 6);
        assert!(app.autosave_tick(35.0).is_some());
        assert!(!store.exists());
        assert_eq!(app.autosave_tick(65.0), None);
        assert!(store.exists());
        // Nothing more to write until the next change.
        std::fs::remove_file(store.project_path()).unwrap();
        assert_eq!(app.autosave_tick(200.0), None);
        assert!(!store.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn steady_changes_are_written_after_the_longest_wait() {
        let dir = temp("steady");
        let mut app = app_with_store(&dir);
        let mut now = 1.0;
        while now < MAX_SECS {
            paint(&mut app, (now as i32) % 50, 7);
            assert!(app.autosave_tick(now).is_some());
            now += 10.0;
        }
        assert!(!Store::at(&dir).exists());
        paint(&mut app, 60, 7);
        assert_eq!(app.autosave_tick(MAX_SECS + 2.0), None);
        assert!(Store::at(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_deletes_the_recovery_file() {
        let dir = temp("save");
        let out = temp("save-out");
        let mut app = app_with_store(&dir);
        paint(&mut app, 1, 5);
        app.autosave_tick(0.0);
        app.autosave_tick(IDLE_SECS + 1.0);
        let store = Store::at(&dir);
        assert!(store.exists());
        app.save_picture(out.join("Pic.dpxproj"), false, false);
        assert!(!store.exists());
        assert!(!app.unsaved());
        // Exporting is not a save.
        paint(&mut app, 2, 6);
        app.autosave_tick(100.0);
        app.autosave_tick(100.0 + IDLE_SECS + 1.0);
        assert!(store.exists());
        app.save_picture(out.join("Pic.iff"), false, true);
        assert!(store.exists());
        assert!(app.unsaved());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn quitting_keeps_unsaved_changes_and_clears_a_clean_slot() {
        let dir = temp("quit");
        let store = Store::at(&dir);
        let mut app = app_with_store(&dir);
        paint(&mut app, 1, 5);
        let ctx = egui::Context::default();
        app.request_quit(&ctx);
        let req = app.ui.choice_requester.as_ref().expect("asks first");
        assert_eq!(req.lines(), ["Quit without saving?"]);
        assert!(!store.exists());
        // Return and Esc answer No.
        app.choice_event(ReqEvent::Key(ReqKey::Enter), &ctx);
        assert!(app.ui.choice_requester.is_none() && !app.autosave.quit_confirmed);
        app.request_quit(&ctx);
        app.choice_event(ReqEvent::Key(ReqKey::Escape), &ctx);
        assert!(!app.autosave.quit_confirmed);
        // Yes (left button) confirms and writes the recovery project.
        app.request_quit(&ctx);
        let req = app.ui.choice_requester.as_ref().unwrap();
        let (x, y) = req.button_centre(0);
        app.choice_event(ReqEvent::Press(x, y), &ctx);
        app.choice_event(ReqEvent::Release, &ctx);
        assert!(app.autosave.quit_confirmed);
        assert!(store.exists());
        // A clean quit empties the slot.
        let mut clean = app_with_store(&dir);
        clean.choice_event(ReqEvent::Key(ReqKey::Escape), &ctx);
        clean.finish_session();
        assert!(!store.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn recovered_pair(dir: &std::path::Path) -> (PainterApp, Vec<u8>) {
        let mut first = app_with_store(dir);
        first.picture_path = Some(PathBuf::from("/pics/Castle.dpxproj"));
        paint(&mut first, 1, 5);
        paint(&mut first, 9, 17);
        first.finish_session();
        let (bytes, _) = files::encode_document(
            first.engine.document(),
            &first.extras,
            &PathBuf::from("x.dpxproj"),
        )
        .unwrap();
        (first, bytes)
    }

    #[test]
    fn recover_round_trip_is_identical_and_still_unsaved() {
        let dir = temp("recover");
        let (first, bytes) = recovered_pair(&dir);
        let mut second = PainterApp::new();
        second.attach_recovery(Store::at(&dir));
        let req = second.ui.choice_requester.as_ref().expect("asks");
        assert_eq!(req.title(), "Recover Picture");
        assert!(
            req.lines().join(" ").contains("Castle"),
            "{:?}",
            req.lines()
        );
        assert_eq!(req.labels(), ["Recover", "Discard"]);
        let ctx = egui::Context::default();
        second.choice_event(ReqEvent::Key(ReqKey::Enter), &ctx);
        let (a, b) = (second.engine.document(), first.engine.document());
        assert_eq!(
            (a.page(), a.palette(), a.target()),
            (b.page(), b.palette(), b.target())
        );
        assert_eq!(a.page().get(1, 3), Some(5));
        let (again, _) = files::encode_document(
            second.engine.document(),
            &second.extras,
            &PathBuf::from("x.dpxproj"),
        )
        .unwrap();
        assert_eq!(again, bytes);
        assert_eq!(
            second.picture_path,
            Some(PathBuf::from("/pics/Castle.dpxproj"))
        );
        assert!(second.unsaved());
        assert!(Store::at(&dir).exists(), "kept until saved");
        second.finish_session();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discard_deletes_and_a_quit_while_asking_keeps_the_file() {
        let dir = temp("discard");
        recovered_pair(&dir);
        let store = Store::at(&dir);
        let ctx = egui::Context::default();
        let mut second = PainterApp::new();
        second.attach_recovery(Store::at(&dir));
        second.finish_session();
        assert!(store.exists(), "the owner has not answered yet");
        second.choice_event(ReqEvent::Key(ReqKey::Escape), &ctx);
        assert!(!store.exists());
        assert!(!second.unsaved());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_recovery_file_is_discarded_with_a_note() {
        let dir = temp("broken");
        let store = Store::at(&dir);
        store
            .write(
                b"not a project",
                &Meta {
                    picture: None,
                    saved_unix: 1,
                },
            )
            .unwrap();
        let mut app = PainterApp::new();
        app.attach_recovery(Store::at(&dir));
        assert!(app.ui.choice_requester.is_none());
        assert!(app.ui.note_requester.is_some());
        assert!(!store.exists());

        // The same when the project breaks between the start and the answer.
        let (_first, _) = recovered_pair(&dir);
        let mut app = PainterApp::new();
        app.attach_recovery(Store::at(&dir));
        std::fs::write(store.project_path(), b"damaged").unwrap();
        let ctx = egui::Context::default();
        app.choice_event(ReqEvent::Key(ReqKey::Enter), &ctx);
        assert!(app.ui.note_requester.is_some());
        assert!(!store.exists());
        assert!(!app.unsaved());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_store_nothing_is_written() {
        let mut app = PainterApp::new();
        paint(&mut app, 1, 5);
        assert_eq!(app.autosave_tick(0.0), None);
        assert_eq!(app.autosave_tick(1000.0), None);
        app.finish_session();
    }
}
