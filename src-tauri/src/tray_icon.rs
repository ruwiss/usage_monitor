use crate::settings::Settings;
use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use image::{Rgba, RgbaImage};
use std::io::Cursor;
use std::sync::LazyLock;

const ICON_SIZE: u32 = 64;
const BAR_HEIGHT: i32 = 9;
const BAR_GAP: i32 = 3;
const MARKER_WIDTH: i32 = 4;
const NUMBER_ROW_HEIGHT: i32 = 32;

static FONT: LazyLock<Vec<u8>> = LazyLock::new(load_font_bytes);
static SYMBOL_FONT: LazyLock<Vec<u8>> = LazyLock::new(load_symbol_font_bytes);

#[derive(Clone, Copy)]
enum VAlign {
    Top,
    Center,
}

#[allow(clippy::too_many_arguments)]
pub fn create_icon_png(
    pct_top: f64,
    pct_bottom: f64,
    light: bool,
    extra_available: bool,
    error_glyph: Option<&str>,
    settings: &Settings,
    mode_top: &str,
    mode_bottom: &str,
    time_pct_top: Option<f64>,
    time_pct_bottom: Option<f64>,
) -> Vec<u8> {
    let colors = if light { &settings.icon_dark } else { &settings.icon_light };
    let fg = rgba(colors.get("fg").copied().unwrap_or([255, 255, 255, 255]));
    let fg_half = rgba(colors.get("fg_half").copied().unwrap_or([255, 255, 255, 80]));
    let fg_warn = rgba(colors.get("fg_warn").copied().unwrap_or([224, 80, 80, 255]));
    let fg_dim = rgba(colors.get("fg_dim").copied().unwrap_or([255, 255, 255, 140]));
    let mut img = RgbaImage::new(ICON_SIZE, ICON_SIZE);
    if let Some(g) = error_glyph {
        draw_text(&mut img, g, fg_dim, 46.0, 0, ICON_SIZE as i32, 0, false, VAlign::Center);
        return encode(&img);
    }
    if settings.icon_style == "numbers" {
        if pct_top >= 100.0 && pct_bottom >= 100.0 {
            if extra_available {
                draw_text(&mut img, "$", fg, 42.0, 0, ICON_SIZE as i32, 2, false, VAlign::Center);
            } else {
                draw_text(&mut img, "\u{2715}", fg, 36.0, 0, ICON_SIZE as i32, 2, true, VAlign::Center);
            }
        } else {
            draw_number_row(&mut img, 0, pct_top, extra_available, fg);
            draw_number_row(&mut img, NUMBER_ROW_HEIGHT, pct_bottom, extra_available, fg);
        }
        return encode(&img);
    }
    let exhausted = pct_top >= 100.0 || pct_bottom >= 100.0;
    if exhausted && !extra_available {
        draw_text(&mut img, "\u{2715}", fg, 36.0, 0, ICON_SIZE as i32, 2, true, VAlign::Top);
    } else if exhausted {
        draw_text(&mut img, "$", fg, 42.0, 0, ICON_SIZE as i32, 2, false, VAlign::Top);
    } else if pct_top > 0.0 {
        draw_text(
            &mut img,
            &format!("{:.0}", pct_top.min(99.0)),
            fg,
            40.0,
            0,
            ICON_SIZE as i32,
            0,
            false,
            VAlign::Top,
        );
    }
    let bar2_y = ICON_SIZE as i32 - BAR_HEIGHT;
    let bar1_y = bar2_y - BAR_GAP - BAR_HEIGHT;
    draw_bar(&mut img, bar1_y, pct_top, mode_top, time_pct_top, fg, fg_half, fg_warn);
    draw_bar(&mut img, bar2_y, pct_bottom, mode_bottom, time_pct_bottom, fg, fg_half, fg_warn);
    encode(&img)
}

fn draw_number_row(img: &mut RgbaImage, row_top: i32, pct: f64, extra_available: bool, fg: Rgba<u8>) {
    if pct >= 100.0 && !extra_available {
        draw_text(img, "\u{2715}", fg, 34.0, row_top, NUMBER_ROW_HEIGHT, 2, true, VAlign::Center);
    } else if pct >= 100.0 {
        draw_text(img, "$", fg, 32.0, row_top, NUMBER_ROW_HEIGHT, 1, false, VAlign::Center);
    } else {
        draw_text(
            img,
            &format!("{:.0}", pct.min(99.0)),
            fg,
            40.0,
            row_top,
            NUMBER_ROW_HEIGHT,
            0,
            false,
            VAlign::Center,
        );
    }
}

fn rgba(c: [u8; 4]) -> Rgba<u8> {
    Rgba(c)
}

fn load_font_bytes() -> Vec<u8> {
    let mut paths = Vec::new();
    if let Ok(windir) = std::env::var("WINDIR") {
        paths.push(format!("{windir}\\Fonts\\arialbd.ttf"));
        paths.push(format!("{windir}\\Fonts\\arial.ttf"));
        paths.push(format!("{windir}\\Fonts\\segoeui.ttf"));
    }
    paths.extend([
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf".into(),
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".into(),
        "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf".into(),
        "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf".into(),
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf".into(),
        "/System/Library/Fonts/Supplemental/Arial.ttf".into(),
        "/Library/Fonts/Arial Bold.ttf".into(),
        "/Library/Fonts/Arial.ttf".into(),
    ]);
    first_valid_font(paths)
}

fn load_symbol_font_bytes() -> Vec<u8> {
    let mut paths = Vec::new();
    if let Ok(windir) = std::env::var("WINDIR") {
        paths.push(format!("{windir}\\Fonts\\seguisym.ttf"));
        paths.push(format!("{windir}\\Fonts\\seguisym.ttf"));
    }
    paths.extend([
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".into(),
        "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf".into(),
        "/usr/share/fonts/truetype/ubuntu/NotoSansSymbols2-Regular.ttf".into(),
        "/usr/share/fonts/TTF/DejaVuSans.ttf".into(),
        "/System/Library/Fonts/Supplemental/Arial.ttf".into(),
        "/Library/Fonts/Arial.ttf".into(),
        "/System/Library/Fonts/Supplemental/Apple Symbols.ttf".into(),
    ]);
    first_valid_font(paths)
}

fn first_valid_font(paths: Vec<String>) -> Vec<u8> {
    for path in paths {
        if let Ok(bytes) = std::fs::read(&path) {
            if FontRef::try_from_slice(&bytes).is_ok() {
                return bytes;
            }
        }
    }
    Vec::new()
}

fn draw_bar(
    img: &mut RgbaImage,
    y: i32,
    pct: f64,
    mode: &str,
    time_pct: Option<f64>,
    fg: Rgba<u8>,
    fg_half: Rgba<u8>,
    fg_warn: Rgba<u8>,
) {
    let w = ICON_SIZE as i32;
    fill_rect(img, 0, y, w, BAR_HEIGHT, fg_half);
    if mode == "overage" {
        if let Some(time_pct) = time_pct {
            if time_pct >= 100.0 {
                if pct >= 100.0 {
                    fill_rect(img, 0, y, w, BAR_HEIGHT, fg);
                }
                return;
            }
            let overage = (pct - time_pct).max(0.0);
            let fill_ratio = (overage / (100.0 - time_pct)).min(1.0);
            let fill_w = (ICON_SIZE as f64 * fill_ratio) as i32;
            if fill_w > 0 {
                fill_rect(img, 0, y, fill_w, BAR_HEIGHT, fg);
            }
            return;
        }
    }
    let fill_w = ((ICON_SIZE as f64 * pct / 100.0) as i32).clamp(0, w);
    if fill_w > 0 {
        let warn = mode == "utilization" && (pct >= 100.0 || time_pct.map(|t| pct > t).unwrap_or(false));
        fill_rect(img, 0, y, fill_w, BAR_HEIGHT, if warn { fg_warn } else { fg });
    }
    if mode != "utilization" {
        return;
    }
    let Some(time_pct) = time_pct else {
        return;
    };
    let marker_x = ((ICON_SIZE as f64 * time_pct / 100.0) as i32 - MARKER_WIDTH / 2).clamp(0, w - MARKER_WIDTH);
    fill_rect(img, marker_x, y, MARKER_WIDTH, BAR_HEIGHT, fg);
}

fn fill_rect(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, color: Rgba<u8>) {
    for dx in 0..w {
        for dy in 0..h {
            let px = x + dx;
            let py = y + dy;
            if px < 0 || py < 0 {
                continue;
            }
            let (px, py) = (px as u32, py as u32);
            if px < ICON_SIZE && py < ICON_SIZE {
                img.put_pixel(px, py, color);
            }
        }
    }
}

fn draw_text(
    img: &mut RgbaImage,
    text: &str,
    fg: Rgba<u8>,
    px: f32,
    box_top: i32,
    box_height: i32,
    stroke: i32,
    symbol: bool,
    align: VAlign,
) {
    let bytes = if symbol && !SYMBOL_FONT.is_empty() {
        SYMBOL_FONT.as_slice()
    } else {
        FONT.as_slice()
    };
    if bytes.is_empty() {
        draw_fallback(img, text, fg, box_top, box_height, align);
        return;
    }
    let Ok(font) = FontRef::try_from_slice(bytes) else {
        draw_fallback(img, text, fg, box_top, box_height, align);
        return;
    };
    let scale = PxScale::from(px);
    let scaled = font.as_scaled(scale);
    let mut glyphs = Vec::new();
    let mut x = 0.0f32;
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    let mut any_outline = false;
    for c in text.chars() {
        let gid = font.glyph_id(c);
        let glyph = gid.with_scale_and_position(scale, point(x, 0.0));
        if let Some(outlined) = font.outline_glyph(glyph) {
            any_outline = true;
            let bounds = outlined.px_bounds();
            min_x = min_x.min(bounds.min.x);
            min_y = min_y.min(bounds.min.y);
            max_x = max_x.max(bounds.max.x);
            max_y = max_y.max(bounds.max.y);
            glyphs.push(outlined);
        }
        x += scaled.h_advance(gid);
    }
    if !any_outline {
        draw_fallback(img, text, fg, box_top, box_height, align);
        return;
    }
    let ink_w = max_x - min_x;
    let ink_h = max_y - min_y;
    let ox = (ICON_SIZE as f32 - ink_w) / 2.0 - min_x;
    let oy = match align {
        VAlign::Top => box_top as f32 - min_y,
        VAlign::Center => box_top as f32 + (box_height as f32 - ink_h) / 2.0 - min_y,
    };
    let mut offsets = vec![(0i32, 0i32)];
    if stroke > 0 {
        offsets.clear();
        for dy in -stroke..=stroke {
            for dx in -stroke..=stroke {
                offsets.push((dx, dy));
            }
        }
    }
    for outlined in &glyphs {
        let bounds = outlined.px_bounds();
        outlined.draw(|gx, gy, coverage| {
            if coverage <= 0.05 {
                return;
            }
            for (dx, dy) in &offsets {
                let px = bounds.min.x + gx as f32 + ox + *dx as f32;
                let py = bounds.min.y + gy as f32 + oy + *dy as f32;
                blit(img, px.round() as i32, py.round() as i32, fg, coverage);
            }
        });
    }
}

fn blit(img: &mut RgbaImage, px: i32, py: i32, fg: Rgba<u8>, coverage: f32) {
    if px < 0 || py < 0 {
        return;
    }
    let (px, py) = (px as u32, py as u32);
    if px >= ICON_SIZE || py >= ICON_SIZE {
        return;
    }
    let src = img.get_pixel(px, py).0;
    let a = (coverage * fg.0[3] as f32) as u8;
    let out = [
        blend(src[0], fg.0[0], a),
        blend(src[1], fg.0[1], a),
        blend(src[2], fg.0[2], a),
        src[3].max(a),
    ];
    img.put_pixel(px, py, Rgba(out));
}

fn blend(dst: u8, src: u8, a: u8) -> u8 {
    let a = a as u16;
    ((src as u16 * a + dst as u16 * (255 - a)) / 255) as u8
}

/// Thick 8×14 digits so a missing TTF still survives 16px tray downscale.
fn draw_fallback(img: &mut RgbaImage, text: &str, fg: Rgba<u8>, box_top: i32, box_height: i32, align: VAlign) {
    const GW: i32 = 8;
    const GH: i32 = 14;
    const GAP: i32 = 2;
    let n = text.chars().count() as i32;
    let total = n * GW + (n - 1).max(0) * GAP;
    let mut x0 = ((ICON_SIZE as i32 - total) / 2).max(0);
    let glyph_h = GH * 2;
    let y0 = match align {
        VAlign::Top => box_top,
        VAlign::Center => box_top + ((box_height - glyph_h) / 2).max(0),
    };
    for c in text.chars() {
        for (x, y) in fallback_glyph(c) {
            for dx in 0..2 {
                for dy in 0..2 {
                    let px = (x0 + x + dx) as u32;
                    let py = (y0 + y + dy) as u32;
                    if px < ICON_SIZE && py < ICON_SIZE {
                        img.put_pixel(px, py, fg);
                    }
                }
            }
        }
        x0 += GW + GAP;
    }
}

fn fallback_glyph(c: char) -> &'static [(i32, i32)] {
    match c {
        '0' => &[
            (1, 0),
            (2, 0),
            (3, 0),
            (4, 0),
            (0, 1),
            (5, 1),
            (0, 2),
            (5, 2),
            (0, 3),
            (5, 3),
            (0, 4),
            (5, 4),
            (0, 5),
            (5, 5),
            (0, 6),
            (5, 6),
            (0, 7),
            (5, 7),
            (0, 8),
            (5, 8),
            (0, 9),
            (5, 9),
            (1, 10),
            (2, 10),
            (3, 10),
            (4, 10),
        ],
        '1' => &[
            (2, 0),
            (1, 1),
            (2, 1),
            (2, 2),
            (2, 3),
            (2, 4),
            (2, 5),
            (2, 6),
            (2, 7),
            (2, 8),
            (2, 9),
            (0, 10),
            (1, 10),
            (2, 10),
            (3, 10),
            (4, 10),
        ],
        '2' => &[
            (1, 0),
            (2, 0),
            (3, 0),
            (4, 0),
            (0, 1),
            (5, 1),
            (5, 2),
            (5, 3),
            (4, 4),
            (3, 5),
            (2, 6),
            (1, 7),
            (0, 8),
            (0, 9),
            (0, 10),
            (1, 10),
            (2, 10),
            (3, 10),
            (4, 10),
            (5, 10),
        ],
        '3' => &[
            (1, 0),
            (2, 0),
            (3, 0),
            (4, 0),
            (0, 1),
            (5, 1),
            (5, 2),
            (5, 3),
            (2, 4),
            (3, 4),
            (4, 4),
            (5, 5),
            (5, 6),
            (5, 7),
            (5, 8),
            (0, 9),
            (5, 9),
            (1, 10),
            (2, 10),
            (3, 10),
            (4, 10),
        ],
        '4' => &[
            (0, 0),
            (4, 0),
            (0, 1),
            (4, 1),
            (0, 2),
            (4, 2),
            (0, 3),
            (4, 3),
            (0, 4),
            (1, 4),
            (2, 4),
            (3, 4),
            (4, 4),
            (5, 4),
            (4, 5),
            (4, 6),
            (4, 7),
            (4, 8),
            (4, 9),
            (4, 10),
        ],
        '5' => &[
            (0, 0),
            (1, 0),
            (2, 0),
            (3, 0),
            (4, 0),
            (5, 0),
            (0, 1),
            (0, 2),
            (0, 3),
            (0, 4),
            (1, 4),
            (2, 4),
            (3, 4),
            (4, 4),
            (5, 5),
            (5, 6),
            (5, 7),
            (5, 8),
            (0, 9),
            (5, 9),
            (1, 10),
            (2, 10),
            (3, 10),
            (4, 10),
        ],
        '6' => &[
            (1, 0),
            (2, 0),
            (3, 0),
            (4, 0),
            (0, 1),
            (0, 2),
            (0, 3),
            (0, 4),
            (1, 4),
            (2, 4),
            (3, 4),
            (4, 4),
            (0, 5),
            (5, 5),
            (0, 6),
            (5, 6),
            (0, 7),
            (5, 7),
            (0, 8),
            (5, 8),
            (0, 9),
            (5, 9),
            (1, 10),
            (2, 10),
            (3, 10),
            (4, 10),
        ],
        '7' => &[
            (0, 0),
            (1, 0),
            (2, 0),
            (3, 0),
            (4, 0),
            (5, 0),
            (5, 1),
            (4, 2),
            (4, 3),
            (3, 4),
            (3, 5),
            (2, 6),
            (2, 7),
            (2, 8),
            (2, 9),
            (2, 10),
        ],
        '8' => &[
            (1, 0),
            (2, 0),
            (3, 0),
            (4, 0),
            (0, 1),
            (5, 1),
            (0, 2),
            (5, 2),
            (0, 3),
            (5, 3),
            (1, 4),
            (2, 4),
            (3, 4),
            (4, 4),
            (0, 5),
            (5, 5),
            (0, 6),
            (5, 6),
            (0, 7),
            (5, 7),
            (0, 8),
            (5, 8),
            (0, 9),
            (5, 9),
            (1, 10),
            (2, 10),
            (3, 10),
            (4, 10),
        ],
        '9' => &[
            (1, 0),
            (2, 0),
            (3, 0),
            (4, 0),
            (0, 1),
            (5, 1),
            (0, 2),
            (5, 2),
            (0, 3),
            (5, 3),
            (0, 4),
            (5, 4),
            (1, 5),
            (2, 5),
            (3, 5),
            (4, 5),
            (5, 5),
            (5, 6),
            (5, 7),
            (5, 8),
            (5, 9),
            (1, 10),
            (2, 10),
            (3, 10),
            (4, 10),
        ],
        '!' => &[(2, 0), (2, 1), (2, 2), (2, 3), (2, 4), (2, 5), (2, 6), (2, 7), (2, 10)],
        '$' => &[
            (2, 0),
            (0, 1),
            (1, 1),
            (2, 1),
            (3, 1),
            (4, 1),
            (0, 2),
            (2, 3),
            (2, 4),
            (1, 5),
            (2, 5),
            (3, 5),
            (4, 6),
            (2, 7),
            (0, 8),
            (1, 8),
            (2, 8),
            (3, 8),
            (4, 8),
            (2, 10),
        ],
        'X' | 'x' | '\u{2715}' => &[
            (0, 0),
            (5, 0),
            (1, 1),
            (4, 1),
            (2, 2),
            (3, 3),
            (2, 4),
            (1, 5),
            (4, 5),
            (0, 6),
            (5, 6),
            (0, 8),
            (5, 8),
            (0, 10),
            (5, 10),
        ],
        _ => &[(2, 5)],
    }
}

fn encode(img: &RgbaImage) -> Vec<u8> {
    let mut buf = Vec::new();
    let _ = img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    fn png(pct_top: f64, pct_bottom: f64, extra: bool, style: &str, mode_top: &str, mode_bottom: &str, t_top: Option<f64>, t_bottom: Option<f64>) -> RgbaImage {
        let mut s = Settings::default();
        s.icon_style = style.into();
        let bytes = create_icon_png(pct_top, pct_bottom, false, extra, None, &s, mode_top, mode_bottom, t_top, t_bottom);
        image::load_from_memory(&bytes).unwrap().to_rgba8()
    }

    fn err_png() -> RgbaImage {
        let s = Settings::default();
        let bytes = create_icon_png(0.0, 0.0, false, false, Some("!"), &s, "utilization", "utilization", None, None);
        image::load_from_memory(&bytes).unwrap().to_rgba8()
    }

    fn bar_mids() -> (u32, u32) {
        let bar2_y = ICON_SIZE as i32 - BAR_HEIGHT;
        let bar1_y = bar2_y - BAR_GAP - BAR_HEIGHT;
        ((bar1_y + BAR_HEIGHT / 2) as u32, (bar2_y + BAR_HEIGHT / 2) as u32)
    }

    fn px(img: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
        img.get_pixel(x, y).0
    }

    #[test]
    fn png_bytes() {
        let s = Settings::default();
        let a = create_icon_png(0.0, 0.0, false, false, None, &s, "utilization", "utilization", None, None);
        let b = create_icon_png(64.0, 10.0, false, false, None, &s, "utilization", "utilization", None, None);
        assert!(a.len() > 50 && b.len() > 50);
        assert_eq!(&a[..8], b"\x89PNG\r\n\x1a\n");
        assert!(b.len() > a.len());
    }

    #[test]
    fn numbers_style_has_no_bars() {
        let img = png(50.0, 50.0, false, "numbers", "utilization", "utilization", Some(40.0), Some(40.0));
        for y in [48u32, 59] {
            assert_eq!(px(&img, 0, y)[3], 0, "bar track at y={y}");
        }
    }

    #[test]
    fn overage_mode_fills_over_budget() {
        let img = png(75.0, 75.0, false, "number+bars", "overage", "overage", Some(50.0), Some(50.0));
        let (m1, m2) = bar_mids();
        let fg = [255, 255, 255, 255];
        let half = [255, 255, 255, 80];
        for mid in [m1, m2] {
            assert_eq!(px(&img, 16, mid), fg);
            assert_eq!(px(&img, 48, mid), half);
        }
    }

    #[test]
    fn time_marker_present_on_utilization_bar() {
        let img = png(20.0, 10.0, false, "number+bars", "utilization", "utilization", Some(50.0), Some(50.0));
        let (m1, m2) = bar_mids();
        let fg = [255, 255, 255, 255];
        assert_eq!(px(&img, 32, m1), fg);
        assert_eq!(px(&img, 32, m2), fg);
    }

    #[test]
    fn warn_when_pct_ahead_of_time() {
        let img = png(70.0, 70.0, false, "number+bars", "utilization", "utilization", Some(40.0), Some(40.0));
        let (m1, _) = bar_mids();
        assert_eq!(px(&img, 5, m1), [224, 80, 80, 255]);
        assert_eq!(px(&img, 24, m1), [255, 255, 255, 255]);
    }

    #[test]
    fn exhausted_blocked_differs_from_dollar() {
        let cross = png(100.0, 20.0, false, "number+bars", "utilization", "utilization", None, None);
        let dollar = png(100.0, 20.0, true, "number+bars", "utilization", "utilization", None, None);
        assert_ne!(cross.as_raw(), dollar.as_raw());
    }

    #[test]
    fn numbers_both_exhausted_no_bars() {
        let img = png(100.0, 100.0, false, "numbers", "utilization", "utilization", None, None);
        for y in [48u32, 59] {
            assert_eq!(px(&img, 0, y)[3], 0);
        }
    }

    #[test]
    fn error_glyph_uses_fg_dim_no_bars() {
        let img = err_png();
        let (m1, m2) = bar_mids();
        assert_eq!(px(&img, 0, m1)[3], 0);
        assert_eq!(px(&img, 0, m2)[3], 0);
        let mut saw = false;
        let mut max_a = 0u8;
        for p in img.pixels() {
            max_a = max_a.max(p.0[3]);
            if p.0[3] > 0 {
                saw = true;
                assert!(p.0[3] <= 140, "error ! must use fg_dim alpha, got {:?}", p.0);
            }
        }
        assert!(saw);
        assert!(max_a > 0 && max_a <= 140);
    }

    #[test]
    fn overage_stale_window_empty_unless_exhausted() {
        let empty = png(80.0, 80.0, false, "number+bars", "overage", "overage", Some(100.0), Some(100.0));
        let full = png(100.0, 100.0, false, "number+bars", "overage", "overage", Some(100.0), Some(100.0));
        let (m1, _) = bar_mids();
        assert_eq!(px(&empty, 32, m1), [255, 255, 255, 80]);
        assert_eq!(px(&full, 32, m1), [255, 255, 255, 255]);
    }
}
