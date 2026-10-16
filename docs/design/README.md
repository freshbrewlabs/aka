# aka design

Final mark: geometric single-story `a` + symmetric `k`; the `k` carries the brand
amber. Letterforms are vector paths (no font dependency) — strokes 15u on a 94u
x-height grid, round caps/joins. The wordmark sits on viewBox `-12 24 243 94`
(width = second bowl `cx` + 62, which holds the leading/trailing bearings at
14.5u/12.5u); the `k` monogram crops that same grid to `90.5 28 52 88`.

Optical spacing: the `a`→`k` slot is 15u of white, uniform because both stems are
parallel and full-height. The `k`→`a` junction is a wedge, not a slot — the arms
sweep up and away from the bowl — so it has no single value: 10.6u at its nearest
(bowl to upper arm), 20.7u averaged over the x-height.

The bowl and the monogram crop share one margin. The crop is pinned between the
`k`'s ink (`x 92.5 → 140.5`) and the bowl outer edge (`cx - 37.5 = 143.5`), which
leaves 2u of crop padding around the `k` and 1u of clearance from the bowl. Below
`cx 180` no crop can exclude the bowl and a sliver of it shows in `favicon.svg` —
visible even at 16px. The square-tab favicon is height-bound (88u), so tightening
the crop's width does not change how the `k` renders there.

| file | use |
|------|-----|
| `aka-logo.svg` | wordmark on light backgrounds (README light mode) |
| `aka-logo-dark.svg` | wordmark on dark backgrounds (README dark mode) |
| `favicon.svg` | `k` monogram, transparent — tabs, small sizes |

## palette

| token | hex |
|-------|-----|
| amber (accent) | `#E07B27` |
| ink | `#1A1B20` |
| paper | `#F7F5F0` |

Sites reference the same amber: `src/public/index.html` and
`src/admin/admin-web/style.css` (`--accent`).

## art, not build inputs

Nothing compiles these files: no `data-trunk` asset, no `include_bytes!`, no
Dockerfile `COPY`, no compose mount. They are copied into code by hand, so a
change here is a manual edit of the copy too:

| file | duplicated in |
|------|---------------|
| `aka-logo-dark.svg` | the admin sidebar `Logo` component (`src/admin/admin-web/src/components/logo/mod.rs`), where the ink letterforms are `currentColor` (the theme's text colour) instead of the paper hex |
| `favicon.svg` | the `data:image/svg+xml` favicons of `src/admin/admin-web/index.html` and `src/public/index.html` — same markup, with double quotes collapsed to single and the amber hex lowercased, as a data URI needs |

Only the root `README.md` reads any of them by path — its header images.
