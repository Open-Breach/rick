# Fixtures of the icon previewer

Hand-made, a few hundred bytes each; every file is read by
`tests/icons.rs`.

- `multi.ico` — three entries: a 32 × 32 PNG (a red disc), a 16 × 16
  32-bit BMP (a green disc) and an 8 × 8 4-bit BMP (a blue square with a
  yellow diagonal inside a transparent frame), in that order in the file.
- `single.ico` — one 16 × 16 PNG entry, a blue disc.
- `arrow.cur` — one 16 × 16 32-bit BMP cursor, a black arrow, hotspot
  (3, 4).
- `truncated.ico` — `multi.ico` cut 40 bytes short.
- `garbage.ico` — a valid directory whose one entry is neither PNG nor
  BMP.
- `mixed.icns` — `ic07` (a 128 × 128 PNG, a blue disc), `is32` with its
  `s8mk` mask (16 × 16 run-length encoded, an orange disc), `ic08`
  carrying a JPEG 2000 signature, and an element of the unknown type
  `zzzz`.
- `named.xpm` — 8 × 4, two characters a pixel, the colours `None`,
  `red`, `light slate blue` and `#00FF00` (with an `m` key to ignore).
