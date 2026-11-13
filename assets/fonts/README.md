# Bundled fonts

The Modern UI skin (ADR-47) draws its text with the Go font family, bundled so that text has
the same metrics on every platform and in tests. No system font is used.

| File | Family | Version (name table) | Licence |
|---|---|---|---|
| `Go-Regular.ttf` | Go Regular | 2.010 | BSD-3-Clause style, see `LICENSE-Go.txt` |
| `Go-Medium.ttf` | Go Medium | 2.010 | BSD-3-Clause style, see `LICENSE-Go.txt` |

Copyright (c) 2016 Bigelow & Holmes Inc. The fonts are unmodified.

`LICENSE-Go.txt` is the licence text stored in the fonts themselves (name table, name ID 13;
identical in both files). A test in `crates/dpx-ui-modern` reads that record from the bundled
files and compares it with `LICENSE-Go.txt`, so a replaced font file cannot bring another
licence unnoticed.

Only fonts under MIT, Apache-2.0, BSD, Zlib or 0BSD terms may be added here (no OFL, no Ubuntu
Font Licence).
