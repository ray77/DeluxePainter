//! Tiny text format for 1-bit pixel art (fonts, icons).
//!
//! ```text
//! # comment
//! = name
//! .##.
//! #..#
//! ```
//! `#` is a set pixel, `.` a clear one. All rows of one entry must have the same width.

/// A 1-bit bitmap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bitmap {
    pub width: usize,
    pub height: usize,
    bits: Vec<bool>,
}

impl Bitmap {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            bits: vec![false; width * height],
        }
    }

    pub fn get(&self, x: usize, y: usize) -> bool {
        x < self.width && y < self.height && self.bits[y * self.width + x]
    }

    pub fn set(&mut self, x: usize, y: usize, on: bool) {
        if x < self.width && y < self.height {
            self.bits[y * self.width + x] = on;
        }
    }
}

/// Parses all `= name` entries of a bitmap text file.
pub fn parse(src: &str) -> Result<Vec<(String, Bitmap)>, String> {
    let mut out: Vec<(String, Bitmap)> = Vec::new();
    let mut current: Option<(String, Vec<Vec<bool>>)> = None;

    let mut finish = |entry: Option<(String, Vec<Vec<bool>>)>| -> Result<(), String> {
        if let Some((name, rows)) = entry {
            let width = rows.first().map_or(0, Vec::len);
            if rows.is_empty() || rows.iter().any(|r| r.len() != width) {
                return Err(format!("entry '{name}': rows missing or of unequal width"));
            }
            let mut bmp = Bitmap::new(width, rows.len());
            for (y, row) in rows.iter().enumerate() {
                for (x, &on) in row.iter().enumerate() {
                    bmp.set(x, y, on);
                }
            }
            out.push((name, bmp));
        }
        Ok(())
    };

    for (lineno, raw) in src.lines().enumerate() {
        let line = raw.trim_end();
        if line.is_empty() || line.starts_with('#') && !line.chars().all(|c| c == '#' || c == '.') {
            continue;
        }
        if let Some(name) = line.strip_prefix('=') {
            finish(current.take())?;
            current = Some((name.trim().to_string(), Vec::new()));
            continue;
        }
        let row: Option<Vec<bool>> = line
            .chars()
            .map(|c| match c {
                '#' => Some(true),
                '.' => Some(false),
                _ => None,
            })
            .collect();
        match (row, current.as_mut()) {
            (Some(row), Some((_, rows))) => rows.push(row),
            _ => return Err(format!("line {}: unexpected content '{line}'", lineno + 1)),
        }
    }
    finish(current.take())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_entries_and_comments() {
        let src = "# header\n= a\n#.\n.#\n\n= b\n##\n";
        let e = parse(src).unwrap();
        assert_eq!(e.len(), 2);
        assert!(e[0].1.get(0, 0) && !e[0].1.get(1, 0) && e[0].1.get(1, 1));
        assert_eq!((e[1].1.width, e[1].1.height), (2, 1));
    }

    #[test]
    fn rows_made_only_of_hashes_are_pixels_not_comments() {
        let e = parse("= full\n###\n#.#\n###\n").unwrap();
        assert_eq!(e[0].1.height, 3);
    }

    #[test]
    fn rejects_ragged_rows() {
        assert!(parse("= x\n##\n#\n").is_err());
    }
}
