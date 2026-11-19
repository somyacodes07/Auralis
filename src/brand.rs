//! Auralis brand mark: the single source of truth for its geometry and colors.
//!
//! The mark is an "A" whose crossbar is a one-cycle soundwave, drawn as round-capped,
//! round-joined strokes on a 64-unit grid (origin top-left, y down). Platform code
//! (the Windows icon and header, the macOS menu-bar template image) and
//! `scripts/generate_brand_assets.py` all read these values; see `docs/BRAND.md`.
//!
//! This module is pure Rust with no platform types so it can be unit tested anywhere.

/// Side length of the design grid every coordinate below is expressed in.
pub const GRID: f32 = 64.0;

/// Stroke width of both polylines, in grid units.
pub const STROKE_WIDTH: f32 = 4.0;

/// The two legs of the "A": left foot, apex, right foot.
pub const A_STROKE: [(f32, f32); 3] = [(13.0, 51.0), (32.0, 13.0), (51.0, 51.0)];

/// The soundwave crossbar, leg to leg: `y = 39 - k * sin(2 pi t) * sin(pi t)` sampled at
/// 21 points, with `k` chosen for a crest and trough 4 units off the bar. The `sin(pi t)`
/// window makes the wave leave each leg horizontally, so it reads as a crossbar; it is
/// point-symmetric about `(32, 39)`.
pub const WAVE_STROKE: [(f32, f32); 21] = [
    (19.0, 39.0),
    (20.3, 38.75),
    (21.6, 38.06),
    (22.9, 37.09),
    (24.2, 36.1),
    (25.5, 35.33),
    (26.8, 35.0),
    (28.1, 35.25),
    (29.4, 36.1),
    (30.7, 37.41),
    (32.0, 39.0),
    (33.3, 40.59),
    (34.6, 41.9),
    (35.9, 42.75),
    (37.2, 43.0),
    (38.5, 42.67),
    (39.8, 41.9),
    (41.1, 40.91),
    (42.4, 39.94),
    (43.7, 39.25),
    (45.0, 39.0),
];

/// Inset of the rounded app tile from the grid edge, in grid units.
pub const TILE_INSET: f32 = 2.0;

/// Corner radius of the rounded app tile, in grid units.
pub const TILE_RADIUS: f32 = 14.0;

/// Tile fill, `#181819`.
pub const TILE_COLOR: [u8; 3] = [0x18, 0x18, 0x19];

/// Mark stroke color, `#f2f2f3`.
pub const MARK_COLOR: [u8; 3] = [0xf2, 0xf2, 0xf3];

/// Thinnest stroke the rasterizer draws, in output pixels, so tiny icons stay legible.
pub const MIN_STROKE_PX: f32 = 1.5;

/// What the mark is drawn on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backdrop {
    /// The rounded graphite app tile; pixels outside it are transparent.
    Tile,
    /// No backdrop: the mark alone with straight alpha.
    Transparent,
    /// An opaque solid color, for compositing into an already painted surface.
    Solid([u8; 3]),
}

/// Distance from `point` to the segment `start..end`.
#[must_use]
pub fn point_segment_distance(point: (f32, f32), start: (f32, f32), end: (f32, f32)) -> f32 {
    let (px, py) = point;
    let (sx, sy) = start;
    let dx = end.0 - sx;
    let dy = end.1 - sy;
    let length_squared = dx * dx + dy * dy;
    let projection = if length_squared <= f32::EPSILON {
        0.0
    } else {
        (((px - sx) * dx + (py - sy) * dy) / length_squared).clamp(0.0, 1.0)
    };
    let nearest_x = sx + projection * dx;
    let nearest_y = sy + projection * dy;
    ((px - nearest_x).powi(2) + (py - nearest_y).powi(2)).sqrt()
}

/// Distance in grid units from `point` to the nearest centerline of the mark.
#[must_use]
pub fn mark_distance(point: (f32, f32)) -> f32 {
    A_STROKE
        .windows(2)
        .chain(WAVE_STROKE.windows(2))
        .map(|segment| point_segment_distance(point, segment[0], segment[1]))
        .fold(f32::INFINITY, f32::min)
}

/// Signed distance in grid units to the rounded tile edge (negative inside).
#[must_use]
pub fn tile_signed_distance(point: (f32, f32)) -> f32 {
    let center = GRID / 2.0;
    let half = center - TILE_INSET - TILE_RADIUS;
    let qx = (point.0 - center).abs() - half;
    let qy = (point.1 - center).abs() - half;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - TILE_RADIUS
}

/// Bounding box `(left, top, right, bottom)` of the inked mark, including the stroke.
#[must_use]
pub fn mark_bounds() -> (f32, f32, f32, f32) {
    let half = STROKE_WIDTH / 2.0;
    A_STROKE.iter().chain(WAVE_STROKE.iter()).fold(
        (
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ),
        |(l, t, r, b), &(x, y)| {
            (
                l.min(x - half),
                t.min(y - half),
                r.max(x + half),
                b.max(y + half),
            )
        },
    )
}

fn coverage(signed_distance_px: f32) -> f32 {
    (0.5 - signed_distance_px).clamp(0.0, 1.0)
}

fn blend(under: [u8; 3], over: [u8; 3], alpha: f32) -> [u8; 3] {
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * alpha).round() as u8;
    [
        mix(under[0], over[0]),
        mix(under[1], over[1]),
        mix(under[2], over[2]),
    ]
}

/// Rasterize the mark into a `size`x`size` RGBA buffer (straight alpha, rows top to bottom).
///
/// Coverage is computed analytically from the distance fields above, which keeps edges
/// anti-aliased at any size. Strokes never drop below [`MIN_STROKE_PX`].
#[must_use]
pub fn render_mark(size: usize, backdrop: Backdrop) -> Vec<u8> {
    let mut pixels = vec![0u8; size * size * 4];
    if size == 0 {
        return pixels;
    }
    let px_per_unit = size as f32 / GRID;
    let stroke_radius_px = (STROKE_WIDTH * px_per_unit).max(MIN_STROKE_PX) / 2.0;
    for y in 0..size {
        for x in 0..size {
            let point = (
                (x as f32 + 0.5) / px_per_unit,
                (y as f32 + 0.5) / px_per_unit,
            );
            let ink = coverage(mark_distance(point) * px_per_unit - stroke_radius_px);
            let (rgb, alpha) = match backdrop {
                Backdrop::Tile => {
                    let tile = coverage(tile_signed_distance(point) * px_per_unit);
                    let ink = ink.min(tile);
                    (blend(TILE_COLOR, MARK_COLOR, ink), tile)
                }
                Backdrop::Transparent => (MARK_COLOR, ink),
                Backdrop::Solid(color) => (blend(color, MARK_COLOR, ink), 1.0),
            };
            let offset = (y * size + x) * 4;
            pixels[offset..offset + 3].copy_from_slice(&rgb);
            pixels[offset + 3] = (alpha * 255.0).round() as u8;
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance_at(pixels: &[u8], size: usize, x: usize, y: usize) -> u8 {
        pixels[(y * size + x) * 4]
    }

    #[test]
    fn mark_sits_inside_the_tile_with_clear_space() {
        let (left, top, right, bottom) = mark_bounds();
        // At least 8 grid units between the inked mark and the tile edge on every side.
        let tile_min = TILE_INSET;
        let tile_max = GRID - TILE_INSET;
        assert!(left - tile_min >= 8.0 && tile_max - right >= 8.0);
        assert!(top - tile_min >= 8.0 && tile_max - bottom >= 8.0);
        // Horizontally centered, vertically within one unit of center.
        assert!(((left + right) / 2.0 - GRID / 2.0).abs() < 0.01);
        assert!(((top + bottom) / 2.0 - GRID / 2.0).abs() <= 1.0);
    }

    #[test]
    fn wave_crossbar_starts_and_ends_on_the_legs() {
        let first = WAVE_STROKE[0];
        let last = WAVE_STROKE[WAVE_STROKE.len() - 1];
        let left_leg = point_segment_distance(first, A_STROKE[0], A_STROKE[1]);
        let right_leg = point_segment_distance(last, A_STROKE[1], A_STROKE[2]);
        assert!(
            left_leg < 0.01,
            "wave start is {left_leg} units off the left leg"
        );
        assert!(
            right_leg < 0.01,
            "wave end is {right_leg} units off the right leg"
        );
        assert!(WAVE_STROKE
            .iter()
            .all(|&(x, _)| x >= first.0 && x <= last.0));
    }

    #[test]
    fn zero_length_segments_measure_point_distance() {
        let distance = point_segment_distance((2.0, 3.0), (1.0, 1.0), (1.0, 1.0));
        assert!((distance - 5.0_f32.sqrt()).abs() < 0.001);
    }

    #[test]
    fn tile_render_has_transparent_corners_and_opaque_center() {
        for size in [16, 24, 32, 48] {
            let pixels = render_mark(size, Backdrop::Tile);
            assert_eq!(pixels.len(), size * size * 4);
            assert_eq!(
                pixels[3], 0,
                "top-left corner must be transparent at {size}px"
            );
            let center = (size / 2 * size + size / 2) * 4;
            assert_eq!(pixels[center + 3], 255);
        }
    }

    #[test]
    fn small_renders_keep_full_contrast_strokes() {
        for size in [16, 20, 24, 32] {
            let pixels = render_mark(size, Backdrop::Tile);
            let brightest = pixels.chunks(4).map(|p| p[0]).max().unwrap();
            let darkest = pixels
                .chunks(4)
                .filter(|p| p[3] == 255)
                .map(|p| p[0])
                .min()
                .unwrap();
            assert!(brightest >= 220, "{size}px stroke peaks at {brightest}");
            assert_eq!(darkest, TILE_COLOR[0]);
        }
    }

    #[test]
    fn apex_and_crossbar_are_inked_and_the_counter_is_open() {
        let size = 64;
        let pixels = render_mark(size, Backdrop::Solid([0, 0, 0]));
        assert_eq!(luminance_at(&pixels, size, 32, 14), MARK_COLOR[0]);
        assert_eq!(luminance_at(&pixels, size, 20, 39), MARK_COLOR[0]);
        // Inside the A, above the wave peak: background.
        assert_eq!(luminance_at(&pixels, size, 32, 24), 0);
        // Below the crossbar between the legs, away from the trough: background.
        assert_eq!(luminance_at(&pixels, size, 26, 46), 0);
    }

    /// Numbers inside the brace-delimited initializer that follows `name` in Objective-C source.
    fn objc_floats(source: &str, name: &str) -> Vec<f32> {
        let start = source.find(name).expect("constant present") + name.len();
        let rest = &source[start..];
        let end = rest.find(';').expect("initializer terminated");
        rest[..end]
            .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .filter(|token| !token.is_empty())
            .map(|token| token.parse().expect("numeric literal"))
            .collect()
    }

    fn flatten(points: &[(f32, f32)]) -> Vec<f32> {
        points.iter().flat_map(|&(x, y)| [x, y]).collect()
    }

    #[test]
    fn macos_template_image_matches_brand_geometry() {
        // The menu-bar image is drawn in Objective-C; keep its copy of the mark identical.
        let source = include_str!("platform/macos_native.m");
        assert_eq!(
            objc_floats(source, "kAuralisMarkA[][2] ="),
            flatten(&A_STROKE)
        );
        assert_eq!(
            objc_floats(source, "kAuralisMarkWave[][2] ="),
            flatten(&WAVE_STROKE)
        );
        assert_eq!(
            objc_floats(source, "kAuralisMarkStrokeWidth ="),
            vec![STROKE_WIDTH]
        );
    }

    #[test]
    fn transparent_render_only_inks_the_mark() {
        let pixels = render_mark(32, Backdrop::Transparent);
        assert!(pixels.chunks(4).any(|p| p[3] == 255));
        assert_eq!(pixels[3], 0);
        assert!(pixels.chunks(4).all(|p| p[..3] == MARK_COLOR));
    }
}
