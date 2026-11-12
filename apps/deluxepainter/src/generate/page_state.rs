//! What a page has outside its document (ADR-45): the colour ranges, grab point and
//! transparent colour (`PictureExtras`), the file name, the fixed background and the BG
//! colour.
//!
//! `Command::ReplacePicture` replaces a page's picture, format and palette in one undo step
//! (Picture ▸ Generate.. onto the picture or the spare page, Delete This Page and Copy to
//! Spare with a spare of its own format). The engine's undo brings back the document only, so
//! this module keeps what the page had outside it with each such step: Undo on that page puts
//! it back, Redo sets what the step gave again. The shown page and the spare page each have
//! their own history, which `j` swaps with the pages (like their engines).
//!
//! The pair of pages either shares its extras (an ordinary spare: one palette, one set of
//! colour ranges) or each page has its own (`MenuData::spare_own`: a spare of its own format
//! and palette). An entry says which it was, so Undo and Redo also split or join them again.
//!
//! An entry is matched to its step by a fingerprint of the command: a `ReplacePicture` this
//! module did not record (one sent over MCP, say) is undone without touching the files, and an
//! entry whose step is gone (the engine was replaced, or dropped its oldest steps) is never
//! taken for another one.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;

use dpx_core::Command;

use crate::PainterApp;
use crate::files::PictureExtras;
use crate::menu_actions::{FixedBackground, OtherPage};

/// One page's state outside its document.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PageFiles {
    /// The page's colour ranges, grab point and transparent colour.
    pub(crate) extras: PictureExtras,
    /// The page shared its extras with the other page (an ordinary spare, or no spare).
    pub(crate) shared: bool,
    pub(crate) picture_path: Option<PathBuf>,
    pub(crate) background: Option<FixedBackground>,
    pub(crate) bg: u8,
}

#[derive(Clone, Debug)]
struct Entry {
    /// [`step_key`] of the step.
    key: u64,
    files: PageFiles,
}

/// One page's entries: before each recorded step (for Undo) and after each undone one (for
/// Redo), most recent last.
#[derive(Clone, Debug, Default)]
pub(crate) struct PageHistory {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
}

impl PageHistory {
    /// Most entries kept each way (the engine keeps at most 64 steps; these steps copy whole
    /// pages, so far fewer stay within its byte budget).
    const MAX: usize = 16;

    fn push(list: &mut Vec<Entry>, entry: Entry) {
        list.push(entry);
        if list.len() > Self::MAX {
            list.remove(0);
        }
    }

    /// The step `key` ran on the page, which had `before`; a new step clears the redo list
    /// (as the engine does).
    pub(crate) fn record(&mut self, key: u64, before: PageFiles) {
        self.redo.clear();
        Self::push(&mut self.undo, Entry { key, files: before });
    }

    /// The step `key` was undone; the page has `now`. The files to put back, if the step is
    /// the one recorded last.
    fn undone(&mut self, key: u64, now: PageFiles) -> Option<PageFiles> {
        if self.undo.last()?.key != key {
            return None;
        }
        let before = self.undo.pop()?.files;
        Self::push(&mut self.redo, Entry { key, files: now });
        Some(before)
    }

    /// The step `key` was redone; the page has `now`. The files it gave, if it is the one
    /// undone last.
    fn redone(&mut self, key: u64, now: PageFiles) -> Option<PageFiles> {
        if self.redo.last()?.key != key {
            return None;
        }
        let after = self.redo.pop()?.files;
        Self::push(&mut self.undo, Entry { key, files: now });
        Some(after)
    }
}

/// The histories of the two pages; `j` swaps them.
#[derive(Clone, Debug, Default)]
pub(crate) struct Histories {
    pub(crate) shown: PageHistory,
    pub(crate) spare: PageHistory,
}

impl Histories {
    pub(crate) fn swap(&mut self) {
        std::mem::swap(&mut self.shown, &mut self.spare);
    }
}

/// The fingerprint of a `ReplacePicture` command (`None` for any other command).
pub(crate) fn step_key(command: &Command) -> Option<u64> {
    let Command::ReplacePicture {
        target,
        palette,
        width,
        height,
        pixels,
    } = command
    else {
        return None;
    };
    let mut h = DefaultHasher::new();
    (target, palette, width, height, pixels).hash(&mut h);
    Some(h.finish())
}

impl PainterApp {
    /// The shown page's files.
    pub(crate) fn shown_files(&self) -> PageFiles {
        PageFiles {
            extras: self.extras.clone(),
            shared: self.menu_data.spare_own.is_none(),
            picture_path: self.picture_path.clone(),
            background: self.menu_data.background.clone(),
            bg: self.ui.bg,
        }
    }

    /// The spare page's files (`other`, taken out of `MenuData::spare`).
    pub(crate) fn spare_files(&self, other: &OtherPage) -> PageFiles {
        PageFiles {
            extras: self
                .menu_data
                .spare_own
                .clone()
                .unwrap_or_else(|| self.extras.clone()),
            shared: self.menu_data.spare_own.is_none(),
            picture_path: other.picture_path.clone(),
            background: other.background.clone(),
            bg: other.bg,
        }
    }

    /// Puts `f` back on the shown page; the pair shares its extras again or splits them as
    /// `f` says. They are shared again only while both pages have one format and palette:
    /// a spare of another format must keep its own (an ordinary spare of another format is
    /// dropped at the next `j`).
    fn restore_shown_files(&mut self, f: PageFiles) {
        self.picture_path = f.picture_path;
        self.menu_data.background = f.background;
        self.ui.bg = f.bg;
        let alike = self.menu_data.spare.as_ref().is_none_or(|s| {
            crate::menu_actions::same_format(self.engine.document(), s.engine.document())
        });
        match (f.shared, self.menu_data.spare_own.take()) {
            // Shared again: the other page's own extras are the shared ones (they may have
            // been edited since).
            (true, Some(other)) if alike => self.extras = other,
            (true, Some(other)) => {
                self.extras = f.extras;
                self.menu_data.spare_own = Some(other);
            }
            (true, None) => self.extras = f.extras,
            (false, Some(other)) => {
                self.extras = f.extras;
                self.menu_data.spare_own = Some(other);
            }
            // Split again: the other page keeps the extras shared until now.
            (false, None) => {
                if self.menu_data.spare.is_some() {
                    self.menu_data.spare_own = Some(std::mem::take(&mut self.extras));
                }
                self.extras = f.extras;
            }
        }
        self.menu_data.cycled = None;
        self.dirty = true;
    }

    /// After an Undo on the shown page: a recorded `ReplacePicture` step brings back the
    /// page's files too.
    pub(crate) fn page_files_undone(&mut self) {
        let Some(key) = self.engine.next_redo().and_then(|(c, _)| step_key(c)) else {
            return;
        };
        let now = self.shown_files();
        if let Some(before) = self.menu_data.page_history.shown.undone(key, now) {
            self.restore_shown_files(before);
        }
    }

    /// The key of the step the next Redo replays (taken before it runs).
    pub(crate) fn redo_key(&self) -> Option<u64> {
        self.engine.next_redo().and_then(|(c, _)| step_key(c))
    }

    /// After a Redo of the step `key` ([`Self::redo_key`]) on the shown page: a recorded
    /// `ReplacePicture` step sets the files it gave again.
    pub(crate) fn page_files_redone(&mut self, key: Option<u64>) {
        let Some(key) = key else {
            return;
        };
        let now = self.shown_files();
        if let Some(after) = self.menu_data.page_history.shown.redone(key, now) {
            self.restore_shown_files(after);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(n: u16) -> PageFiles {
        PageFiles {
            extras: PictureExtras {
                ranges: Vec::new(),
                grab: None,
                transparent_color: Some(n),
            },
            shared: true,
            picture_path: None,
            background: None,
            bg: n as u8,
        }
    }

    #[test]
    fn entries_follow_their_own_steps_only() {
        let mut h = PageHistory::default();
        h.record(1, files(1));
        h.record(2, files(2));
        // Another step (not recorded) is undone: nothing to put back.
        assert_eq!(h.undone(9, files(0)), None);
        assert_eq!(h.undone(2, files(20)), Some(files(2)));
        assert_eq!(h.redone(9, files(0)), None);
        assert_eq!(h.redone(2, files(3)), Some(files(20)));
        assert_eq!(h.undone(2, files(4)), Some(files(3)));
        // A new step clears what Redo would have given.
        h.record(5, files(5));
        assert_eq!(h.redone(2, files(0)), None);
        assert_eq!(h.undone(5, files(6)), Some(files(5)));
        assert_eq!(h.undone(1, files(7)), Some(files(1)));
        assert_eq!(h.undone(1, files(7)), None, "nothing left");
    }

    #[test]
    fn the_history_is_bounded() {
        let mut h = PageHistory::default();
        for k in 0..40 {
            h.record(k, files(k as u16));
        }
        assert_eq!(h.undo.len(), PageHistory::MAX);
        assert_eq!(h.undone(39, files(0)), Some(files(39)));
    }

    #[test]
    fn only_replace_picture_has_a_key_and_it_follows_the_content() {
        let command = |p: u8| Command::ReplacePicture {
            target: dpx_core::Target::default(),
            palette: vec![dpx_core::Rgb8::BLACK, dpx_core::Rgb8::WHITE],
            width: 1,
            height: 1,
            pixels: vec![p],
        };
        assert_eq!(step_key(&command(0)), step_key(&command(0)));
        assert_ne!(step_key(&command(0)), step_key(&command(1)));
        assert_eq!(step_key(&Command::Clear { color: 0 }), None);
    }
}
