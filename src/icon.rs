//! Draws the tray icon: a coloured rounded square with the battery percentage.

pub const SIZE: usize = 32;

/// 3x5 pixel glyphs, one row per entry, bit 2 = left column.
fn glyph(c: char) -> [u8; 5] {
    match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b001, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        '-' => [0b000, 0b000, 0b111, 0b000, 0b000],
        _ => [0b111, 0b001, 0b011, 0b000, 0b010], // '?'
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

const GREEN: Rgb = Rgb(0x2E, 0x9E, 0x44);
const AMBER: Rgb = Rgb(0xD9, 0x8E, 0x04);
const RED: Rgb = Rgb(0xD9, 0x30, 0x25);
const GREY: Rgb = Rgb(0x6E, 0x6E, 0x6E);
const BOLT: Rgb = Rgb(0xFF, 0xD4, 0x00);

/// What the icon should show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconState {
    /// None = no device / no value yet.
    pub level: Option<u8>,
    pub charging: bool,
    /// False when the value is the last known one (device asleep): drawn grey.
    pub live: bool,
    /// Devices are present but none gave a value: draws "?" instead of "-".
    pub unanswered: bool,
}

pub fn background(state: &IconState) -> Rgb {
    match state.level {
        Some(_) if !state.live => GREY,
        Some(l) if l < 20 => RED,
        Some(l) if l < 50 => AMBER,
        Some(_) => GREEN,
        None => GREY,
    }
}

/// RGBA pixels, SIZE x SIZE.
pub fn render(state: &IconState) -> Vec<u8> {
    let mut px = vec![0u8; SIZE * SIZE * 4];
    let bg = background(state);
    let radius = 6.0f32;
    let s = SIZE as f32;

    // Rounded square with simple anti-aliased corners.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let cx = fx.clamp(radius, s - radius);
            let cy = fy.clamp(radius, s - radius);
            let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            let alpha = (radius - d + 0.5).clamp(0.0, 1.0);
            put(&mut px, x, y, bg, (alpha * 255.0) as u8);
        }
    }

    let text = match state.level {
        Some(l) => l.min(100).to_string(),
        None if state.unanswered => "?".to_string(),
        None => "-".to_string(),
    };
    let scale = match text.len() {
        1 => 5,
        2 => 4,
        _ => 3,
    };
    let gap = if text.len() == 3 { 1 } else { 4 };
    let w = text.len() * 3 * scale + (text.len() - 1) * gap;
    let h = 5 * scale;
    let x0 = (SIZE - w) / 2;
    let y0 = (SIZE - h) / 2;
    let white = Rgb(0xFF, 0xFF, 0xFF);
    for (i, ch) in text.chars().enumerate() {
        let g = glyph(ch);
        let gx = x0 + i * (3 * scale + gap);
        for (row, bits) in g.iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) != 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            put(&mut px, gx + col * scale + dx, y0 + row * scale + dy, white, 255);
                        }
                    }
                }
            }
        }
    }

    // Charging marker: yellow triangle in the top-right corner.
    if state.charging {
        for y in 0..9 {
            for x in (SIZE - 9 + y)..SIZE {
                put(&mut px, x, y, BOLT, 255);
            }
        }
    }
    px
}

fn put(px: &mut [u8], x: usize, y: usize, c: Rgb, a: u8) {
    if x >= SIZE || y >= SIZE {
        return;
    }
    let i = (y * SIZE + x) * 4;
    px[i] = c.0;
    px[i + 1] = c.1;
    px[i + 2] = c.2;
    px[i + 3] = a;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_all_widths_inside_bounds() {
        for level in [None, Some(0), Some(7), Some(42), Some(100)] {
            for charging in [false, true] {
                let px = render(&IconState { level, charging, live: true, unanswered: false });
                assert_eq!(px.len(), SIZE * SIZE * 4);
            }
        }
    }

    #[test]
    fn colours_follow_level() {
        let st = |level, live| IconState { level, charging: false, live, unanswered: false };
        assert_eq!(background(&st(Some(80), true)), GREEN);
        assert_eq!(background(&st(Some(30), true)), AMBER);
        assert_eq!(background(&st(Some(5), true)), RED);
        assert_eq!(background(&st(Some(80), false)), GREY);
        assert_eq!(background(&st(None, true)), GREY);
    }
}
