# Fonts

`Phosphor.ttf` — Phosphor Icons, "regular" weight (https://phosphoricons.com), MIT licence
(`LICENSE-Phosphor.txt`). Downloaded 2026-10-10 from the project's `phosphor-icons/web`
repository. Used for the generic icons of the interface (file, undo, eye, gear, ...); the
CAD-specific icons are drawn in code (`src/ui/icons.rs`). Its characters live in the Unicode
private use area, so the font is added as a fallback of the normal text fonts and a glyph can be
written straight into a label. The codepoints are in `src/ui/icon_font.rs` (generated from the
font's own `style.css`).
