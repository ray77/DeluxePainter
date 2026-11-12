//! What a running MCP server shares between its transports and its owner (ADR-43): the
//! permissions and approved directories (the GUI changes them live from Prefs > AI & MCP)
//! and the activity counters behind the "connected" status.

use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use crate::access::FileAccess;

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
    /// Read the document (info, palette, pixels, preview, export, save).
    pub read: bool,
    /// Change the document (draw, palette, new, open, import, undo, redo, swap).
    pub edit: bool,
    pub overwrite: Overwrite,
    /// The approved directories for every file path.
    pub access: FileAccess,
}

impl Settings {
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

#[derive(Debug, Default)]
pub struct Activity {
    inner: Mutex<ActivitySnapshot>,
}

impl Activity {
    fn with<R>(&self, f: impl FnOnce(&mut ActivitySnapshot) -> R) -> R {
        f(&mut self.inner.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// One request arrived.
    pub fn request(&self) {
        self.with(|a| {
            a.requests += 1;
            a.last_request = Some(Instant::now());
        });
    }

    pub fn tool_call(&self) {
        self.with(|a| {
            a.tool_calls += 1;
            a.last_request = Some(Instant::now());
        });
    }

    pub fn client(&self, name: &str) {
        let name: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
        if !name.is_empty() {
            self.with(|a| a.client = Some(name));
        }
    }

    pub fn snapshot(&self) -> ActivitySnapshot {
        self.with(|a| a.clone())
    }
}

/// Settings and activity of one server, shared by its transports and its owner.
#[derive(Debug)]
pub struct ServerState {
    settings: RwLock<Settings>,
    activity: Activity,
}

impl ServerState {
    pub fn new(settings: Settings) -> Arc<Self> {
        Arc::new(Self {
            settings: RwLock::new(settings),
            activity: Activity::default(),
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
}
