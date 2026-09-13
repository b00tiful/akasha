use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier},
};

// A single Braille dot occupies a fraction of a terminal cell. Never use full-size stars here:
// the prompt's quiet pixel flecks should stay smaller and dimmer than the text above them.
const DOTS: [[&str; 2]; 4] = [["⠁", "⠈"], ["⠂", "⠐"], ["⠄", "⠠"], ["⡀", "⢀"]];

/// Draw after the prompt, excluding its entire input row (including spaces and the cursor).
/// The caller owns the 20 Hz clock and freezes it when motion is disabled.
pub(super) fn draw(
    frame: &mut Frame,
    area: Rect,
    tick: u64,
    ascii: bool,
    color: bool,
    protected: Option<Rect>,
) {
    paint(frame.buffer_mut(), area, tick, ascii, color, protected);
}

fn paint(
    buffer: &mut Buffer,
    area: Rect,
    tick: u64,
    ascii: bool,
    color: bool,
    protected: Option<Rect>,
) {
    let area = area.intersection(buffer.area);
    if area.width < 3 || area.height == 0 {
        return;
    }
    let columns = u64::from(area.width - 2);
    let rows = u64::from(area.height);
    let count = (columns * rows / 6).clamp(1, 128);
    let wave = (tick as f64 * std::f64::consts::TAU / 120.0).cos();
    for index in 0..count {
        let seed = hash(index.wrapping_add(0x616b_6173_6861));
        let life = 20 + seed % 21; // 1–2 seconds at 20 Hz, including invisible endpoints.
        let age = (tick % life + (seed >> 32) % life) % life;
        let envelope = fade(age, life);
        let column = seed % columns;
        let edge = (column as f64 / columns as f64 * std::f64::consts::TAU).cos();
        // The population brightens at the edges, then at the center, every six seconds.
        // Dots never travel across the band: only their density and luminance roll inward.
        let weight = 0.12 + 0.88 * (0.5 + 0.5 * wave * edge);
        let brightness = (190.0 * envelope * weight) as u8;
        if brightness < 10 {
            continue;
        }
        let row = (seed >> 16) % rows;
        let position = (area.x + 1 + column as u16, area.y + row as u16);
        if protected.is_some_and(|region| region.contains(position.into())) {
            continue;
        }
        let cell = &mut buffer[position];
        if cell.symbol() != " " || cell.modifier.contains(Modifier::REVERSED) {
            continue;
        }
        // One horizontal Braille subcell step only, then back; no cell translation or wrap.
        // The exact pixel distance follows the emulator's font (roughly 2–3 px at review size).
        let side = usize::from(age > life / 3 && age < life * 2 / 3);
        cell.set_symbol(if ascii {
            "."
        } else {
            DOTS[((seed >> 24) % 4) as usize][side]
        });
        if color {
            cell.set_fg(Color::Rgb(brightness, brightness, brightness));
        } else {
            cell.modifier.insert(Modifier::DIM);
        }
    }
}

fn fade(age: u64, life: u64) -> f64 {
    (std::f64::consts::PI * age as f64 / (life - 1) as f64)
        .sin()
        .powi(2)
}

fn hash(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(tick: u64, ascii: bool, color: bool) -> Buffer {
        let area = Rect::new(0, 0, 160, 5);
        let mut buffer = Buffer::empty(area);
        paint(&mut buffer, area, tick, ascii, color, None);
        buffer
    }

    #[test]
    fn fixed_clock_is_stable_and_motion_stays_sparse() {
        let first = sample(0, false, true);
        assert_eq!(first, sample(0, false, true));
        assert_ne!(first, sample(48, false, true));
        for tick in [0, 1, 48, 800, u64::MAX] {
            let buffer = sample(tick, false, true);
            let specks = buffer
                .content
                .iter()
                .filter(|cell| cell.symbol() != " ")
                .count();
            assert!((1..=128).contains(&specks));
            for y in 0..buffer.area.height {
                assert_eq!(buffer[(0, y)].symbol(), " ");
                assert_eq!(buffer[(159, y)].symbol(), " ");
            }
        }
    }

    #[test]
    fn protects_input_spaces_existing_content_and_backgrounds() {
        let area = Rect::new(0, 0, 160, 5);
        let protected = Rect::new(0, 2, 160, 1);
        let mut original = Buffer::empty(area);
        for (index, cell) in original.content.iter_mut().enumerate() {
            cell.bg = Color::Rgb(24, 23, 28);
            cell.fg = Color::Yellow;
            if index % 7 == 0 {
                cell.set_symbol("x");
            }
        }
        original[(90, 0)].modifier = Modifier::REVERSED;
        for tick in 0..96 {
            let mut painted = original.clone();
            paint(&mut painted, area, tick, false, true, Some(protected));
            for position in area.positions() {
                let before = &original[position];
                let after = &painted[position];
                assert_eq!(after.bg, before.bg);
                if protected.contains(position)
                    || before.symbol() != " "
                    || before.modifier.contains(Modifier::REVERSED)
                {
                    assert_eq!(after, before);
                } else if after.symbol() != " " {
                    assert!(
                        DOTS.iter()
                            .flatten()
                            .any(|symbol| *symbol == after.symbol())
                    );
                    let Color::Rgb(red, green, blue) = after.fg else {
                        panic!("colored flecks must use neutral gray");
                    };
                    assert_eq!((red, red), (green, blue));
                    assert!((10..=190).contains(&red));
                }
            }
        }
    }

    #[test]
    fn ascii_and_monochrome_fallbacks_keep_terminal_colors() {
        let buffer = sample(24, true, false);
        assert!(buffer.content.iter().any(|cell| cell.symbol() == "."));
        for cell in &buffer.content {
            assert!(matches!(cell.symbol(), " " | "."));
            assert_eq!(cell.fg, Color::Reset);
            assert_eq!(cell.bg, Color::Reset);
            if cell.symbol() == "." {
                assert!(cell.modifier.contains(Modifier::DIM));
            }
        }
    }

    #[test]
    fn clips_offset_and_tiny_areas_without_overflow() {
        let bounds = Rect::new(7, 11, 30, 4);
        for area in [
            Rect::new(8, 12, 0, 0),
            Rect::new(8, 12, 1, 1),
            Rect::new(8, 12, 2, 1),
            Rect::new(9, 12, 20, 2),
            Rect::new(0, 0, 20, 20),
            Rect::new(u16::MAX - 1, u16::MAX - 1, 1, 1),
        ] {
            let mut buffer = Buffer::empty(bounds);
            paint(&mut buffer, area, u64::MAX, true, false, None);
            for position in bounds.positions() {
                if !area.contains(position) || area.width < 3 {
                    assert_eq!(buffer[position].symbol(), " ");
                }
            }
        }
        let area = Rect::new(0, 0, 400, 80);
        let mut buffer = Buffer::empty(area);
        paint(&mut buffer, area, 0, false, true, None);
        assert!(
            buffer
                .content
                .iter()
                .filter(|cell| cell.symbol() != " ")
                .count()
                <= 128
        );
    }
    #[test]
    fn short_lifetimes_fade_to_zero_without_cell_travel() {
        for life in 20..=40 {
            assert!(fade(0, life) < 0.00001);
            assert!(fade(life - 1, life) < 0.00001);
            for age in 1..life {
                assert!((fade(age, life) - fade(age - 1, life)).abs() < 0.17);
            }
        }
        let area = Rect::new(0, 0, 160, 3);
        let mut positions = std::collections::BTreeSet::new();
        for tick in 0..120 {
            let mut b = Buffer::empty(area);
            paint(&mut b, area, tick, false, true, None);
            for p in area.positions() {
                if b[p].symbol() != " " {
                    positions.insert((p.x, p.y));
                }
            }
        }
        assert!(
            positions.len() <= 79,
            "dots must stay in their seeded cells"
        );
    }

    #[test]
    fn luminous_population_alternates_edges_and_center() {
        fn energy(tick: u64) -> (u64, u64) {
            let area = Rect::new(0, 0, 300, 3);
            let mut b = Buffer::empty(area);
            paint(&mut b, area, tick, false, true, None);
            let mut edges = 0;
            let mut center = 0;
            for p in area.positions() {
                if b[p].symbol() == " " {
                    continue;
                }
                if let Color::Rgb(v, _, _) = b[p].fg {
                    if p.x < 75 || p.x >= 225 {
                        edges += u64::from(v);
                    } else {
                        center += u64::from(v);
                    }
                }
            }
            (edges, center)
        }
        let (edge, center) = energy(0);
        assert!(edge > center);
        let (edge, center) = energy(60);
        assert!(center > edge);
        let (edge, center) = energy(120);
        assert!(edge > center);
    }
}
