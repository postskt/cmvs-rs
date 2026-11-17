//! Short host feedback (quick save/load results, an unavailable menu) drawn as a
//! white-on-black box near the bottom of the game canvas. The box is opaque, so the
//! GPU composites it with the same replace-blended scale pass as a movie frame and
//! the software path blits it over the finished surface. Text stays ASCII: the
//! fallback system font is the Japanese shared font, which need not cover Chinese.
use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
use cmvs_console::canvas::Canvas;
use cmvs_core::raster::RgbaImage;
use std::time::{Duration, Instant};

/// How long a toast stays up.
pub const LIFETIME: Duration = Duration::from_millis(2500);

pub struct Toast {
    image: RgbaImage,
    shown: Instant,
}

impl Toast {
    pub fn new(font: &FontVec, text: &str, screen: [u32; 2]) -> Self {
        Self {
            image: render(font, text, screen),
            shown: Instant::now(),
        }
    }

    pub fn alive(&self) -> bool {
        self.shown.elapsed() < LIFETIME
    }

    pub fn image(&self) -> &RgbaImage {
        &self.image
    }
}

/// Rasterizes `text` white on black, sized for a `screen`-tall display: the text
/// height is 1/24 of the screen (30 px at 720p), with half a text height of padding.
/// Overlong text is shortened with an ellipsis so the box always fits the screen.
pub fn render(font: &FontVec, text: &str, screen: [u32; 2]) -> RgbaImage {
    let px = (screen[1] as f32 / 24.0).max(12.0);
    let pad = (px / 2.0).round() as u32;
    let max = screen[0].saturating_sub(pad * 4).max(1);
    let mut line = text.to_owned();
    if Canvas::text_width(font, px, &line) > max as f32 {
        let ellipsis = if font.glyph_id('…').0 != 0 { "…" } else { "..." };
        let mut chars: Vec<char> = text.chars().collect();
        while !chars.is_empty() {
            chars.pop();
            line = chars.iter().collect::<String>() + ellipsis;
            if Canvas::text_width(font, px, &line) <= max as f32 {
                break;
            }
        }
    }
    let scaled = font.as_scaled(PxScale::from(px));
    let width = Canvas::text_width(font, px, &line).ceil() as u32;
    let ascent = scaled.ascent().ceil();
    let descent = scaled.descent().floor();
    let height = (ascent - descent).ceil() as u32;
    let (w, h) = (width + pad * 2, height + pad * 2);
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    for alpha in pixels.iter_mut().skip(3).step_by(4) {
        *alpha = 255;
    }
    let mut caret = pad as f32;
    let baseline = pad as f32 + ascent;
    let mut previous = None;
    for ch in line.chars() {
        let id = scaled.glyph_id(ch);
        if let Some(previous) = previous {
            caret += scaled.kern(previous, id);
        }
        let glyph = id.with_scale_and_position(px, point(caret, baseline));
        caret += scaled.h_advance(id);
        previous = Some(id);
        if let Some(outline) = font.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                let x = bounds.min.x as i32 + gx as i32;
                let y = bounds.min.y as i32 + gy as i32;
                if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                    return;
                }
                let at = (y as usize * w as usize + x as usize) * 4;
                let value = (coverage.clamp(0.0, 1.0) * 255.0).round() as u8;
                pixels[at] = value;
                pixels[at + 1] = value;
                pixels[at + 2] = value;
            });
        }
    }
    RgbaImage::new(w, h, pixels).expect("toast-sized image")
}

/// Blits the opaque toast over a software surface at the bottom-left margin.
pub fn blit(surface: &mut [u32], screen: [u32; 2], image: &RgbaImage) {
    let margin = (screen[1] / 24).max(12);
    let left = margin;
    let top = screen[1].saturating_sub(image.height() + margin);
    for y in 0..image.height() {
        let row = top + y;
        if row >= screen[1] {
            break;
        }
        for x in 0..image.width() {
            let column = left + x;
            if column >= screen[0] {
                break;
            }
            let at = (y * image.width() + x) as usize * 4;
            let p = &image.pixels()[at..at + 4];
            surface[(row * screen[0] + column) as usize] =
                0xff000000 | u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2]);
        }
    }
}

/// The canvas-space bottom-left position matching [`blit`].
pub fn position(screen: [u32; 2], image: &RgbaImage) -> [u32; 2] {
    let margin = (screen[1] / 24).max(12);
    [margin, screen[1].saturating_sub(image.height() + margin)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font() -> FontVec {
        let bytes = std::fs::read(std::env::var("CMVS_MENU_FONT").expect("CMVS_MENU_FONT")).unwrap();
        FontVec::try_from_vec_and_index(bytes, 0).unwrap()
    }

    #[test]
    #[ignore = "needs a font file: CMVS_MENU_FONT=<font> cargo test -p cmvs-switch toast -- --ignored"]
    fn a_toast_is_opaque_white_text_on_black() {
        let image = render(&font(), "Quick save OK [860]", [1280, 720]);
        assert!(image.width() < 1280 && image.height() < 120);
        let pixels = image.pixels();
        assert!(pixels.chunks(4).all(|p| p[3] == 255), "opaque box");
        assert!(pixels.chunks(4).any(|p| p[0] > 200), "white text");
        assert!(pixels[..4] == [0, 0, 0, 255], "black corner");
    }

    #[test]
    #[ignore = "needs a font file: CMVS_MENU_FONT=<font> cargo test -p cmvs-switch toast -- --ignored"]
    fn overlong_text_is_shortened_to_fit() {
        let long = "x".repeat(4000);
        let image = render(&font(), &long, [1280, 720]);
        assert!(image.width() <= 1280);
    }

    #[test]
    fn blit_and_position_agree_and_stay_on_screen() {
        // A synthetic box: no font needed for the placement contract.
        let image = RgbaImage::new(40, 20, vec![255; 40 * 20 * 4]).unwrap();
        let at = position([1280, 720], &image);
        let mut surface = vec![0u32; 1280 * 720];
        blit(&mut surface, [1280, 720], &image);
        let top_left = surface[(at[1] * 1280 + at[0]) as usize];
        assert_eq!(top_left, 0xffffffff, "the box corner landed at `position`");
        assert!(at[1] + image.height() <= 720);
        assert_eq!(surface[0], 0, "the rest of the surface is untouched");
    }
}