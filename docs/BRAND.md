# Auralis brand

One memorable element, the mark. Everything around it stays quiet.

## Name

- The product is **Auralis**: one word, capital A, everywhere users can see it (window titles,
  tray and menu-bar tooltips, menus, installers, the macOS bundle, release names, the CLI banner).
- The Windows configuration window is **Auralis Studio**. In running text "the Studio" is fine.
- Do not use "Auralis Engine", "Auralis Live Wallpaper Studio", "Auralis Configuration Studio",
  "Auralis Wallpaper Engine" or other variants. Describe what it does in a sentence instead:
  "Auralis is a live video wallpaper app for Windows and macOS."
- Version strings follow the name: `Auralis v1.0.0-rc.1`, `Auralis Studio · v1.0.0-rc.1`.
- Sentence case for UI copy and artwork ("Drag Auralis to Applications").
- "Project Aurora" is an internal codename in `AGENTS.md`; it never appears in the product.

## Mark

An **A** whose crossbar is a single soundwave, drawn as round-capped, round-joined strokes.

The geometry has one source of truth, [`src/brand.rs`](../src/brand.rs), on a 64-unit grid
(origin top-left, y down):

| Element | Value |
| --- | --- |
| A legs | `(13, 51) → (32, 13) → (51, 51)` (legs drop 2 units for every unit across) |
| Wave crossbar | leg to leg at `y = 39`: `y = 39 − k·sin(2πt)·sin(πt)`, crest `y = 35`, trough `y = 43`, sampled at 21 points |
| Stroke | 4 units, round caps and joins |
| Tile | inset 2, 60 × 60, corner radius 14 |
| Inked mark | 42 × 42 units, centered on the grid |

The wave leaves each leg horizontally and stays between them, so the mark reads as an A first
and a waveform second. Everything that draws the mark uses these numbers:

- Windows title-bar, taskbar and tray icon: `assets/auralis.ico` (embedded as resource 1), or
  `brand::render_mark` when the executable has no icon resource.
- Windows Studio header: `brand::render_mark` over the window background.
- macOS menu-bar template image: `src/platform/macos_native.m` keeps a copy of the coordinates;
  the test `brand::tests::macos_template_image_matches_brand_geometry` fails if they drift.
- All generated artwork: `scripts/generate_brand_assets.py` reads `src/brand.rs`.

## Color tokens

| Token | Hex | Use |
| --- | --- | --- |
| `tile` | `#181819` | App tile, wordmark on light backgrounds |
| `ink` | `#f2f2f3` | The mark; wordmark on dark backgrounds |
| `field` | `#111112` | Installer panel |
| `window` | `#0d0d0e` | Studio window background (header mark backdrop) |
| `dmg-field` | `#f4f4f5` | DMG window background |
| `dmg-arrow` | `#b4b4b8` | DMG arrow |
| `hint` | `#6e6e73` | Secondary text in artwork |

Monochrome only. No accent color in brand art, no gradients, glows, glass or shadows beyond the
platform's standard app-icon shadow on macOS.

## Wordmark

"Auralis" is drawn, not typeset: monoline strokes at the mark's stroke weight (cap height 32,
x-height 22 on the same grid), defined in `wordmark_glyphs()` in the generator. This keeps the
art reproducible on any machine and avoids redistributing font outlines. In the horizontal
lockup (`assets/auralis-logo.svg`, `assets/auralis-logo-dark.svg`) the wordmark's cap height is
half the tile and it sits 24 units to the right of the tile.

In the running app the name is ordinary text in the system UI font (Segoe UI on Windows).

## Clear space and sizes

- Keep clear space of at least a quarter of the tile (16 units) around the tile or lockup.
- Minimum sizes: tile 16 px; bare mark 14 px wide; lockup 96 px wide. Below 24 px the icon
  generator snaps the crossbar to the pixel grid and deepens the wave so it stays distinct.
- Inside the tile the inked mark keeps at least 8 units from the tile edge (unit tested).

## Do / don't

Do: use the tile as the app icon; use the bare mark on the app's own dark surfaces; let the
mark be the only bright element.

Don't: recolor the mark, add effects, stretch or rotate it, put the wave outside the legs,
redraw it with new coordinates in platform code, or set "Auralis" in all caps with tracking.

## Assets

All committed and regenerated with:

```powershell
python scripts/generate_brand_assets.py [--preview DIR]
```

Requires Python 3.10+, Pillow and numpy. The DMG hint is set in Segoe UI
(`C:\Windows\Fonts\segoeui.ttf`, override with `--font`), so regeneration is expected on
Windows. Rerunning with the same inputs reproduces every file byte for byte.

| Output | Purpose |
| --- | --- |
| `assets/auralis-mark.svg`, `assets/auralis-logo.svg`, `assets/auralis-logo-dark.svg` | Vector mark and lockups |
| `assets/auralis.ico` | Windows icon: 16, 20, 24, 32, 40, 48, 64, 128 (32-bit BMP) and 256 (PNG) |
| `assets/macos/AppIcon.iconset/` | macOS icon sources (16–1024 px, Apple's 824/1024 tile grid); CI runs `iconutil` |
| `assets/macos/dmg-background.png`, `@2x` | DMG window art; CI combines them with `tiffutil` |
| `assets/installer/wizard-image*.bmp` | Inno Setup welcome panel, 100/125/150/200 % |
| `assets/installer/wizard-small*.bmp` | Inno Setup page icon, 100/125/150/200 % |

The DMG window is 600 × 400 pt with Auralis.app at (160, 168) and the Applications link at
(440, 168); `package.yml` passes the same positions to `create-dmg`. The DMG background is
light so Finder's default dark icon labels stay readable; in macOS Dark Mode Finder draws
white labels, which have low contrast on it (known limitation).
