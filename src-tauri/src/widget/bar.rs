//! Progress bars for the widget card, drawn as small PNGs.
//!
//! Adaptive Cards has no progress bar, and a styled Column (the usual workaround) gets the
//! host's container padding, which makes it a thick block. A thin image stretched to its
//! column's width keeps the same height on every row.

use base64::Engine;

const WIDTH: u32 = 240;
const HEIGHT: u32 = 12;
/// Supersampling per axis for anti-aliased rounded ends.
const SAMPLES: u32 = 4;

/// Track color: a translucent mid grey reads on both the dark lock screen and light boards.
const TRACK: [u8; 4] = [128, 128, 128, 90];

/// Fill color for an Adaptive Card color name (see `color_for`).
fn fill_for(color: &str) -> [u8; 4] {
    match color {
        "Attention" => [239, 68, 68, 255],
        "Warning" => [245, 166, 35, 255],
        _ => [34, 197, 94, 255],
    }
}

/// `data:` URI of a bar `pct`% full (0–100) in the given color.
pub fn bar_data_uri(pct: f64, color: &str) -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bar_png(pct, color))
    )
}

fn bar_png(pct: f64, color: &str) -> Vec<u8> {
    let fill = fill_for(color);
    let w = WIDTH as f64;
    let h = HEIGHT as f64;
    // A non-zero value always shows at least a round dot.
    let filled = if pct <= 0.0 {
        0.0
    } else {
        (pct.clamp(0.0, 100.0) / 100.0 * w).max(h)
    };

    let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let track = coverage(x, y, w);
            let bar = if filled > 0.0 { coverage(x, y, filled) } else { 0.0 };
            pixels.extend_from_slice(&over(fill, bar, TRACK, track));
        }
    }

    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, WIDTH, HEIGHT);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("in-memory PNG header");
        writer
            .write_image_data(&pixels)
            .expect("in-memory PNG data");
    }
    out
}

/// Fraction of pixel (x, y) inside a capsule from x = 0 to `len`, full image height.
fn coverage(x: u32, y: u32, len: f64) -> f64 {
    let r = HEIGHT as f64 / 2.0;
    let mut inside = 0;
    for sy in 0..SAMPLES {
        for sx in 0..SAMPLES {
            let px = x as f64 + (sx as f64 + 0.5) / SAMPLES as f64;
            let py = y as f64 + (sy as f64 + 0.5) / SAMPLES as f64;
            // Distance from the capsule's center segment [r, len - r] at height r.
            let cx = px.clamp(r, (len - r).max(r));
            let (dx, dy) = (px - cx, py - r);
            if px <= len && dx * dx + dy * dy <= r * r {
                inside += 1;
            }
        }
    }
    inside as f64 / (SAMPLES * SAMPLES) as f64
}

/// `top` at coverage `a` composited over `bottom` at coverage `b` (straight alpha).
fn over(top: [u8; 4], a: f64, bottom: [u8; 4], b: f64) -> [u8; 4] {
    let ta = top[3] as f64 / 255.0 * a;
    let ba = bottom[3] as f64 / 255.0 * b;
    let out_a = ta + ba * (1.0 - ta);
    if out_a <= 0.0 {
        return [0, 0, 0, 0];
    }
    let mix = |i: usize| {
        ((top[i] as f64 * ta + bottom[i] as f64 * ba * (1.0 - ta)) / out_a).round() as u8
    };
    [mix(0), mix(1), mix(2), (out_a * 255.0).round() as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_a_png_of_the_expected_size() {
        let png = bar_png(42.0, "Good");
        assert_eq!(&png[1..4], b"PNG");
        let size = u32::from_be_bytes(png[16..20].try_into().unwrap());
        assert_eq!(size, WIDTH);
    }

    #[test]
    fn fills_proportionally() {
        let mid = HEIGHT / 2;
        // 50%: filled at 25% of the width, track only at 75%.
        assert!(coverage(WIDTH / 4, mid, WIDTH as f64 / 2.0) > 0.99);
        assert_eq!(coverage(WIDTH * 3 / 4, mid, WIDTH as f64 / 2.0), 0.0);
        // Rounded corner: the top-left pixel is mostly outside.
        assert!(coverage(0, 0, WIDTH as f64) < 0.5);
    }
}

