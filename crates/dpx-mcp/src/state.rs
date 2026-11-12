//! What a running MCP server shares between its transports and its owner (ADR-43): the
//! permissions and approved directories (the GUI changes them live from Prefs > AI & MCP),
//! the file encoders ([`Codec`]), the activity counters behind the "connected" status and
//! the activity log of the last tool calls.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

use crate::access::FileAccess;
use crate::codec::{BuiltinCodec, Codec};

/// What `overwrite: true` on `save_project` / `export_image` does with an existing file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Overwrite {
    /// Replace it without asking (the headless server; the GUI's "Overwrite files without
    /// asking" box).
    Allowed,
    /// Ask the user first ([`crate::Workspace::ask`]); a "no" fails the call.
    #[default]
    Ask,
}

/// The permissions of the MCP client.
#[derive(Clone, Debug)]
pub struct Settings {
    /// Read the document (info, palette, pixels, preview, export, save). Off: every tool
    /// except `get_app_status` is refused, changing ones too (they report what they did).
    pub read: bool,
    /// Change the document (draw, palette, new, open, import, undo, redo, swap).
    pub edit: bool,
    pub overwrite: Overwrite,
    /// The approved directories for every file path.
    pub access: FileAccess,
}

impl Settings {
    /// `overwrite: true` replaces an existing file without asking: overwriting changes a
    /// file, so it also needs both permissions (the GUI ghosts the box otherwise).
    pub fn overwrite_without_asking(&self) -> bool {
        self.read && self.edit && self.overwrite == Overwrite::Allowed
    }

    /// The headless server (`deluxepainter mcp`): everything allowed; overwrite only with
    /// `overwrite: true` (ADR-24).
    pub fn headless(access: FileAccess) -> Self {
        Self {
            read: true,
            edit: true,
            overwrite: Overwrite::Allowed,
            access,
        }
    }
}

/// When and by whom the server was last used.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActivitySnapshot {
    /// Authenticated HTTP requests (or stdio tool calls).
    pub requests: u64,
    pub tool_calls: u64,
    pub last_request: Option<Instant>,
    /// The client's name from `initialize` or the request metadata, if it sent one.
    pub client: Option<String>,
}

impl ActivitySnapshot {
    /// A client sent a request within `within` (the status display's "connected": the HTTP
    /// server is stateless, so there is no connection to watch).
    pub fn active_within(&self, within: Duration) -> bool {
        self.last_request.is_some_and(|t| t.elapsed() <= within)
    }
}

/// Lines kept in the activity log.
pub const LOG_ENTRIES: usize = 50;

/// One tool call in the activity log ("what the client did").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    /// When the call finished (wall clock, for the display).
    pub time: SystemTime,
    pub tool: String,
    pub ok: bool,
    /// The error code of a failed call.
    pub code: Option<String>,
    /// What it did, e.g. `draw_rectangle 10,10 20x20 colour 3` (from [`crate::Workspace::log`]),
    /// or the error message.
    pub detail: Option<String>,
}

#[derive(Debug, Default)]
struct ActivityInner {
    snapshot: ActivitySnapshot,
    log: VecDeque<LogEntry>,
    /// What the running call did so far ([`Activity::note`]).
    note: Option<String>,
    /// Bumped with every log entry, so a display can tell when to redraw.
    serial: u64,
}

#[derive(Debug, Default)]
pub struct Activity {
    inner: Mutex<ActivityInner>,
}

impl Activity {
    fn with<R>(&self, f: impl FnOnce(&mut ActivityInner) -> R) -> R {
        f(&mut self.inner.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// One request arrived.
    pub fn request(&self) {
        self.with(|a| {
            a.snapshot.requests += 1;
            a.snapshot.last_request = Some(Instant::now());
        });
    }

    /// A tool call starts (calls run one at a time, ADR-43).
    pub fn tool_call(&self) {
        self.with(|a| {
            a.snapshot.tool_calls += 1;
            a.snapshot.last_request = Some(Instant::now());
            a.note = None;
        });
    }

    pub fn client(&self, name: &str) {
        let name: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
        if !name.is_empty() {
            self.with(|a| a.snapshot.client = Some(name));
        }
    }

    pub fn snapshot(&self) -> ActivitySnapshot {
        self.with(|a| a.snapshot.clone())
    }

    /// What the running call did (the workspace's log line); kept for its log entry.
    pub fn note(&self, detail: &str) {
        let detail: String = detail
            .chars()
            .filter(|c| !c.is_control())
            .take(160)
            .collect();
        self.with(|a| a.note = Some(detail));
    }

    /// The running call finished: one log entry with what it did, or its error.
    pub fn finish(&self, tool: &str, error: Option<(&str, &str)>) {
        self.with(|a| {
            let note = a.note.take();
            if a.log.len() == LOG_ENTRIES {
                a.log.pop_front();
            }
            a.log.push_back(LogEntry {
                time: SystemTime::now(),
                tool: tool.to_string(),
                ok: error.is_none(),
                code: error.map(|(c, _)| c.to_string()),
                detail: match error {
                    None => note,
                    Some((_, msg)) => Some(msg.chars().take(160).collect()),
                },
            });
            a.serial += 1;
        });
    }

    /// The activity log, oldest first.
    pub fn log(&self) -> Vec<LogEntry> {
        self.with(|a| a.log.iter().cloned().collect())
    }

    /// Changes with every new log entry.
    pub fn log_serial(&self) -> u64 {
        self.with(|a| a.serial)
    }
}

/// Settings, encoders and activity of one server, shared by its transports and its owner.
#[derive(Debug)]
pub struct ServerState {
    settings: RwLock<Settings>,
    activity: Activity,
    codec: Arc<dyn Codec>,
}

impl ServerState {
    /// With the built-in encoders ([`BuiltinCodec`], the headless server).
    pub fn new(settings: Settings) -> Arc<Self> {
        Self::with_codec(settings, Arc::new(BuiltinCodec))
    }

    /// With the owner's encoders (the GUI writes files with its own file code).
    pub fn with_codec(settings: Settings, codec: Arc<dyn Codec>) -> Arc<Self> {
        Arc::new(Self {
            settings: RwLock::new(settings),
            activity: Activity::default(),
            codec,
        })
    }

    pub fn settings(&self) -> Settings {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Takes effect with the next call.
    pub fn set_settings(&self, settings: Settings) {
        *self.settings.write().unwrap_or_else(|e| e.into_inner()) = settings;
    }

    pub fn activity(&self) -> &Activity {
        &self.activity
    }

    pub fn codec(&self) -> Arc<dyn Codec> {
        self.codec.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_keeps_the_last_calls_with_what_they_did() {
        let a = Activity::default();
        for i in 0..LOG_ENTRIES + 3 {
            a.tool_call();
            a.note(&format!("draw_pixel {i},0 colour 1"));
            a.finish("draw_pixel", None);
        }
        a.tool_call();
        a.note("stale note of a failed call");
        a.finish(
            "export_image",
            Some(("OVERWRITE_DECLINED", "the user kept a.iff")),
        );
        let log = a.log();
        assert_eq!(log.len(), LOG_ENTRIES);
        assert_eq!(log[0].detail.as_deref(), Some("draw_pixel 4,0 colour 1"));
        let last = log.last().unwrap();
        assert!(!last.ok);
        assert_eq!(last.code.as_deref(), Some("OVERWRITE_DECLINED"));
        assert_eq!(last.detail.as_deref(), Some("the user kept a.iff"));
        assert_eq!(a.log_serial(), LOG_ENTRIES as u64 + 4);
        // A call without a note logs no detail.
        a.tool_call();
        a.finish("get_palette", None);
        assert_eq!(a.log().last().unwrap().detail, None);
        let s = a.snapshot();
        assert_eq!(s.tool_calls, LOG_ENTRIES as u64 + 5);
        assert!(s.active_within(Duration::from_secs(60)));
        assert!(!ActivitySnapshot::default().active_within(Duration::from_secs(60)));
    }
}
