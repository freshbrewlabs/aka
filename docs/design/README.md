# aka design

Final mark: geometric single-story `a` + symmetric `k`; the `k` carries the brand
amber. Letterforms are vector paths (no font dependency) — strokes 15u on a 94u
x-height grid, round caps/joins. The wordmark sits on viewBox `-12 24 252 94`;
the `k` monogram crops that same grid to `86 28 61 88`.

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
| `favicon.svg` | the `data:image/svg+xml` favicons of `src/admin/admin-web/index.html` and `src/public/index.html`, byte for byte |

Only the root `README.md` reads any of them by path — its header images.
