//! Validation and application of the frame commands (ADR-28), kept apart from the page
//! commands in `command.rs`. Frame numbers are 0-based here; the interface counts from 1.
//! A frame is all its layer pictures (ADR-44): frame commands copy, move and delete them
//! together.

use crate::command::{Command, CommandError, ErrorCode};
use crate::document::{Document, check_fps, check_frame_count};

/// Frame `f` must exist.
fn frame_exists(doc: &Document, f: u32, what: &str) -> Result<usize, CommandError> {
    let count = doc.frame_count();
    let i = f as usize;
    if i < count {
        Ok(i)
    } else {
        Err(CommandError::invalid(format!(
            "{what} frame {} is outside 1..={count}",
            u64::from(f) + 1
        )))
    }
}

/// `first..=last` must be existing frames in order; returns them as indices.
fn frame_range(doc: &Document, first: u32, last: u32) -> Result<(usize, usize), CommandError> {
    let a = frame_exists(doc, first, "first")?;
    let b = frame_exists(doc, last, "last")?;
    if a > b {
        return Err(CommandError::invalid(format!(
            "frame range {}..{} is empty",
            a + 1,
            b + 1
        )));
    }
    Ok((a, b))
}

/// A document of `count` frames of the current page size and layers must stay within the
/// limits.
fn room_for(doc: &Document, count: u64) -> Result<(), CommandError> {
    let (w, h) = (doc.page.width(), doc.page.height());
    let count = usize::try_from(count).unwrap_or(usize::MAX);
    check_frame_count(count, doc.layer_count(), w, h)
}

pub(crate) fn validate(cmd: &Command, doc: &Document) -> Result<(), CommandError> {
    let total = doc.frame_count() as u64;
    match *cmd {
        Command::SetFrameCount { count } => room_for(doc, u64::from(count)),
        Command::AddFrames { count } => {
            if count == 0 {
                return Err(CommandError::invalid("add at least one frame".into()));
            }
            room_for(doc, total + u64::from(count))
        }
        Command::CopyFrame { first, last } => frame_range(doc, first, last).map(|_| ()),
        Command::InsertFrameCopies {
            first,
            last,
            before,
        } => {
            let (a, b) = frame_range(doc, first, last)?;
            if u64::from(before) > total {
                return Err(CommandError::invalid(format!(
                    "insert before frame {}: 1 to {} allowed",
                    u64::from(before) + 1,
                    total + 1
                )));
            }
            room_for(doc, total + (b - a + 1) as u64)
        }
        Command::DeleteFrames { first, last, fill } => {
            frame_range(doc, first, last)?;
            if usize::from(fill) >= doc.palette.len() {
                return Err(CommandError::new(
                    ErrorCode::PaletteRangeInvalid,
                    format!(
                        "colour {fill} is outside the palette (0..{})",
                        doc.palette.len()
                    ),
                ));
            }
            Ok(())
        }
        Command::GotoFrame { frame } => frame_exists(doc, frame, "go to").map(|_| ()),
        Command::SetFrameRate { fps } => check_fps(fps),
        _ => unreachable!("not a frame command"),
    }
}

/// Where the pictures go under frame command `cmd` (what [`apply`] does to the frame list;
/// ADR-39: an area stencil follows its picture): for every frame of a document of `count`
/// frames whose current frame is `current`, its index afterwards, `None` if it is deleted.
/// Copies are new pictures. `None` for every command that keeps all frames where they are
/// (CopyFrame repaints frames in place). `cmd` must be valid for such a document.
pub fn frame_moves(cmd: &Command, count: usize, current: usize) -> Option<Vec<Option<usize>>> {
    // `n` new frames go in at `at`.
    let insert = |at: usize, n: usize| {
        (0..count)
            .map(|i| Some(if i < at { i } else { i + n }))
            .collect()
    };
    // `n` frames from `at` on go.
    let delete = |at: usize, n: usize| {
        (0..count)
            .map(|i| match i {
                _ if i < at => Some(i),
                _ if i < at + n => None,
                _ => Some(i - n),
            })
            .collect()
    };
    let span = |first: u32, last: u32| last.saturating_sub(first) as usize + 1;
    match *cmd {
        Command::SetFrameCount { count: to } => {
            let to = to as usize;
            if to > count {
                Some(insert(current + 1, to - count))
            } else if to < count {
                Some(delete(current.min(to), count - to))
            } else {
                None
            }
        }
        Command::AddFrames { count: n } => Some(insert(current + 1, n as usize)),
        Command::InsertFrameCopies {
            first,
            last,
            before,
        } => Some(insert(before as usize, span(first, last))),
        Command::DeleteFrames { first, last, .. } => {
            Some(delete(first as usize, span(first, last)))
        }
        _ => None,
    }
}

pub(crate) fn apply(cmd: &Command, doc: &mut Document) {
    let total = doc.frame_count();
    let current = doc.current;
    match *cmd {
        Command::SetFrameCount { count } => {
            let count = count as usize;
            let mut frames = doc.frame_cels();
            if count > total {
                // Copies of the current frame after it (AGA p.236: "step to the frame you want
                // to add frames after").
                let copy = frames[current].clone();
                let at = current + 1;
                frames.splice(at..at, std::iter::repeat_n(copy, count - total));
                doc.set_frames(frames, current);
            } else if count < total {
                // Delete from the current frame on ("step to frame 41 and set the number of
                // frames to 90" deletes 41 to 50); near the end, the last frames go.
                let n = total - count;
                let start = current.min(count);
                frames.drain(start..start + n);
                doc.set_frames(frames, start.min(count - 1));
            }
        }
        Command::AddFrames { count } => {
            let count = count as usize;
            let mut frames = doc.frame_cels();
            let copy = frames[current].clone();
            let at = current + 1;
            frames.splice(at..at, std::iter::repeat_n(copy, count));
            doc.set_frames(frames, current + count);
        }
        Command::CopyFrame { first, last } => {
            let mut frames = doc.frame_cels();
            let copy = frames[current].clone();
            for f in &mut frames[first as usize..=last as usize] {
                f.clone_from(&copy);
            }
            doc.set_frames(frames, current);
        }
        Command::InsertFrameCopies {
            first,
            last,
            before,
        } => {
            let (a, b, at) = (first as usize, last as usize, before as usize);
            let mut frames = doc.frame_cels();
            let copies: Vec<_> = frames[a..=b].to_vec();
            let n = copies.len();
            frames.splice(at..at, copies);
            // The picture shown stays the current one.
            let current = if at <= current { current + n } else { current };
            doc.set_frames(frames, current);
        }
        Command::DeleteFrames { first, last, fill } => {
            let (a, b) = (first as usize, last as usize);
            let n = b - a + 1;
            if n == total {
                // Nothing is left: one frame in the fill (BG) colour (its upper layers clear).
                let blank = doc.blank_frame(fill);
                doc.set_frames(vec![blank], 0);
                return;
            }
            let mut frames = doc.frame_cels();
            frames.drain(a..=b);
            let left = frames.len();
            let current = if current < a {
                current
            } else if current > b {
                current - n
            } else {
                // The frame after the deleted ones, unless they were the last.
                a.min(left - 1)
            };
            doc.set_frames(frames, current);
        }
        Command::GotoFrame { frame } => doc.go_to(frame as usize),
        Command::SetFrameRate { fps } => doc.fps = fps,
        _ => unreachable!("not a frame command"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IndexedImage, Palette, Target};

    /// Five 2x1 frames; frame i is filled with colour i + 1.
    fn doc(current: usize) -> Document {
        let frames = (0..5).map(|i| IndexedImage::new(2, 1, i + 1)).collect();
        Document::from_frames(
            frames,
            current,
            30,
            Palette::dpaint_default_32(),
            Target::default(),
        )
        .unwrap()
    }

    /// Colours of all frames and the current frame index.
    fn state(d: &Document) -> (Vec<u8>, usize) {
        (
            d.frames().map(|f| f.get(0, 0).unwrap()).collect(),
            d.current_frame(),
        )
    }

    fn run(d: &mut Document, cmd: Command) {
        validate(&cmd, d).unwrap();
        apply(&cmd, d);
    }

    #[test]
    fn set_frame_count_grows_after_and_shrinks_from_the_current_frame() {
        let mut d = doc(1);
        run(&mut d, Command::SetFrameCount { count: 7 });
        assert_eq!(state(&d), (vec![1, 2, 2, 2, 3, 4, 5], 1));
        let mut d = doc(1);
        run(&mut d, Command::SetFrameCount { count: 3 });
        assert_eq!(state(&d), (vec![1, 4, 5], 1), "frames 2 and 3 deleted");
        let mut d = doc(4);
        run(&mut d, Command::SetFrameCount { count: 3 });
        assert_eq!(
            state(&d),
            (vec![1, 2, 3], 2),
            "near the end the last ones go"
        );
        let mut d = doc(2);
        run(&mut d, Command::SetFrameCount { count: 5 });
        assert_eq!(state(&d), (vec![1, 2, 3, 4, 5], 2));
        assert!(validate(&Command::SetFrameCount { count: 0 }, &d).is_err());
        assert!(validate(&Command::SetFrameCount { count: 1000 }, &d).is_err());
    }

    #[test]
    fn add_frames_copies_the_current_one_and_moves_to_the_last_copy() {
        let mut d = doc(1);
        run(&mut d, Command::AddFrames { count: 2 });
        assert_eq!(state(&d), (vec![1, 2, 2, 2, 3, 4, 5], 3));
        assert!(validate(&Command::AddFrames { count: 0 }, &d).is_err());
        assert!(validate(&Command::AddFrames { count: 993 }, &d).is_err());
        validate(&Command::AddFrames { count: 992 }, &d).unwrap();
    }

    #[test]
    fn copy_and_insert_copies() {
        let mut d = doc(1);
        run(&mut d, Command::CopyFrame { first: 3, last: 4 });
        assert_eq!(state(&d), (vec![1, 2, 3, 2, 2], 1));
        run(&mut d, Command::CopyFrame { first: 0, last: 4 });
        assert_eq!(state(&d), (vec![2; 5], 1), "Copy to All");
        assert!(validate(&Command::CopyFrame { first: 3, last: 2 }, &d).is_err());
        assert!(validate(&Command::CopyFrame { first: 0, last: 5 }, &d).is_err());

        let mut d = doc(3);
        let ins = |first, last, before| Command::InsertFrameCopies {
            first,
            last,
            before,
        };
        run(&mut d, ins(0, 1, 2));
        assert_eq!(
            state(&d),
            (vec![1, 2, 1, 2, 3, 4, 5], 5),
            "same picture current"
        );
        run(&mut d, ins(6, 6, 7));
        assert_eq!(state(&d), (vec![1, 2, 1, 2, 3, 4, 5, 5], 5), "at the end");
        assert!(validate(&ins(0, 0, 9), &d).is_err());
    }

    #[test]
    fn delete_frames_picks_the_following_frame() {
        let del = |first, last| Command::DeleteFrames {
            first,
            last,
            fill: 9,
        };
        let mut d = doc(2);
        run(&mut d, del(2, 2));
        assert_eq!(state(&d), (vec![1, 2, 4, 5], 2), "the following frame");
        let mut d = doc(4);
        run(&mut d, del(4, 4));
        assert_eq!(
            state(&d),
            (vec![1, 2, 3, 4], 3),
            "the last frame: the one before"
        );
        let mut d = doc(4);
        run(&mut d, del(0, 1));
        assert_eq!(
            state(&d),
            (vec![3, 4, 5], 2),
            "current moves with its frame"
        );
        let mut d = doc(0);
        run(&mut d, del(3, 4));
        assert_eq!(state(&d), (vec![1, 2, 3], 0));
        let mut d = doc(3);
        run(&mut d, del(0, 4));
        assert_eq!(state(&d), (vec![9], 0), "all: one frame in the fill colour");
        let bad = Command::DeleteFrames {
            first: 0,
            last: 0,
            fill: 40,
        };
        assert_eq!(
            validate(&bad, &d).unwrap_err().code,
            ErrorCode::PaletteRangeInvalid
        );
    }

    #[test]
    fn frame_moves_follow_every_picture_as_apply_moves_it() {
        let commands = [
            Command::SetFrameCount { count: 8 },
            Command::SetFrameCount { count: 2 },
            Command::SetFrameCount { count: 5 },
            Command::AddFrames { count: 2 },
            Command::InsertFrameCopies {
                first: 0,
                last: 1,
                before: 0,
            },
            Command::InsertFrameCopies {
                first: 3,
                last: 3,
                before: 2,
            },
            Command::InsertFrameCopies {
                first: 1,
                last: 4,
                before: 5,
            },
            Command::DeleteFrames {
                first: 1,
                last: 1,
                fill: 0,
            },
            Command::DeleteFrames {
                first: 0,
                last: 2,
                fill: 0,
            },
            Command::DeleteFrames {
                first: 0,
                last: 4,
                fill: 9,
            },
            Command::CopyFrame { first: 0, last: 4 },
            Command::GotoFrame { frame: 3 },
        ];
        for cmd in &commands {
            for current in 0..5 {
                let mut d = doc(current);
                run(&mut d, cmd.clone());
                let after: Vec<u8> = state(&d).0;
                let Some(moves) = frame_moves(cmd, 5, current) else {
                    // In place: as many frames as before.
                    assert_eq!(after.len(), 5, "{cmd:?}");
                    continue;
                };
                assert_eq!(moves.len(), 5);
                // Every kept picture is where the map says; the others are copies.
                for (old, to) in moves.iter().enumerate() {
                    if let Some(to) = *to {
                        assert_eq!(after[to], old as u8 + 1, "{cmd:?} at {current}");
                    }
                }
                let targets: Vec<usize> = moves.iter().flatten().copied().collect();
                assert!(
                    targets.windows(2).all(|w| w[0] < w[1]),
                    "in order, one place each: {cmd:?}"
                );
                assert!(targets.len() <= after.len());
            }
        }
        // A copy goes in before the originals or after them, as the commands say.
        let delete_one = Command::DeleteFrames {
            first: 1,
            last: 1,
            fill: 0,
        };
        assert_eq!(
            frame_moves(&delete_one, 3, 1),
            Some(vec![Some(0), None, Some(1)])
        );
        let insert_first = Command::InsertFrameCopies {
            first: 0,
            last: 0,
            before: 0,
        };
        assert_eq!(
            frame_moves(&insert_first, 3, 1),
            Some(vec![Some(1), Some(2), Some(3)])
        );
        assert_eq!(
            frame_moves(&Command::AddFrames { count: 2 }, 3, 1),
            Some(vec![Some(0), Some(1), Some(4)])
        );
        // The single frame left after deleting all is a new picture.
        let all = Command::DeleteFrames {
            first: 0,
            last: 4,
            fill: 9,
        };
        assert_eq!(frame_moves(&all, 5, 2), Some(vec![None; 5]));
        assert_eq!(
            frame_moves(&Command::SetFrameCount { count: 5 }, 5, 0),
            None
        );
    }

    #[test]
    fn goto_and_rate() {
        let mut d = doc(0);
        run(&mut d, Command::GotoFrame { frame: 4 });
        assert_eq!(state(&d).1, 4);
        assert_eq!(d.page().get(0, 0), Some(5));
        assert!(validate(&Command::GotoFrame { frame: 5 }, &d).is_err());
        run(&mut d, Command::SetFrameRate { fps: 12 });
        assert_eq!(d.fps(), 12);
        assert!(validate(&Command::SetFrameRate { fps: 0 }, &d).is_err());
        assert!(validate(&Command::SetFrameRate { fps: 61 }, &d).is_err());
    }
}
