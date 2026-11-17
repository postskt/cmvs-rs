//! deko3d execution of the shared scene queue through
//! [`cmvs_console::gpu::Compositor`]; movie scaling, presentation and readback are the
//! Switch's own. Readback happens only when the engine requests a device frame.
use crate::sys::SwitchGpu;
use cmvs_console::gpu::Compositor;
use cmvs_core::{
    raster::RgbaImage,
    scene_render::{
        Blend, Layer, RenderFrame,
        shader::{BASE_FLOATS, shader_draw},
    },
    viewport::Viewport,
};
use cmvs_native::GpuDevice;
use std::error::Error;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
/// Retained texture bytes, as in cmvs-wgpu; not a bound on total GPU memory.
const CACHE_LIMIT: usize = 128 * 1024 * 1024;

pub struct GpuRenderer(Compositor<SwitchGpu>);

impl GpuRenderer {
    pub fn new(gpu: SwitchGpu) -> Result<Self> {
        Ok(Self(Compositor::new(gpu, CACHE_LIMIT)?))
    }

    /// Releases every cached texture, e.g. before another game starts.
    pub fn clear_cache(&mut self) {
        self.0.clear_cache();
    }

    pub fn render(&mut self, frame: &RenderFrame<'_>) -> Result<()> {
        self.0.render(frame)
    }

    /// Fits an image (a decoded movie frame, a host screen) onto an opaque canvas of
    /// `size`. A movie frame stays in the frame cache, so a frame shown again (the
    /// display outpaces the movie) is not uploaded again. Host screens bypass the
    /// cache and its digest.
    pub fn render_image(&mut self, image: &RgbaImage, size: [u32; 2], linear: bool) -> Result<()> {
        self.0.begin_frame()?;
        let source = [image.width(), image.height()];
        let view = Viewport::new(source, size, 1)?;
        let (texture, transient) = match image.frame_id() {
            Some(id) => (self.0.frame_texture(id, image)?, false),
            None => (self.0.upload(image)?, true),
        };
        let result = self.0.begin_canvas(size, [0, 0, 0, 255]).and_then(|()| {
            let params = view_params(view, source, linear);
            Ok(self.0.device().scale(texture, params)?)
        });
        if transient {
            self.0.device().release_texture(texture);
        }
        result
    }

    /// Rebuilds the swapchain for a new display size between frames.
    pub fn resize(&mut self, size: [u32; 2]) -> Result<()> {
        Ok(self.0.device().resize(size)?)
    }

    /// Shows the canvas on the `screen`-sized display with the engine's display
    /// `mode`; `cursor` is a screen position for the host crosshair and `toast` an
    /// opaque host box at a screen position. Both are drawn on the display only:
    /// device readback never includes them.
    pub fn present(
        &mut self,
        screen: [u32; 2],
        mode: u32,
        linear: bool,
        cursor: Option<[f64; 2]>,
        toast: Option<(&RgbaImage, [u32; 2])>,
    ) -> Result<()> {
        let size = self.0.canvas().ok_or("no GPU canvas to present")?;
        let view = Viewport::new(size, screen, mode)?;
        let overlays = cursor.map(|at| cursor_overlays(at, screen)).unwrap_or_default();
        let uploaded = match toast {
            Some((image, at)) => {
                let texture = self.0.upload(image)?;
                let (w, h) = (image.width() as f32, image.height() as f32);
                Some((texture, [at[0] as f32, at[1] as f32, w, h, w, h, 0.0, 0.0]))
            }
            None => None,
        };
        let result = self
            .0
            .device()
            .present(view_params(view, size, linear), &overlays, uploaded);
        if let Some((texture, _)) = uploaded {
            self.0.device().release_texture(texture);
        }
        result?;
        self.0.mark_presented();
        Ok(())
    }

    /// The last presented logical canvas.
    pub fn read(&mut self) -> Result<RgbaImage> {
        let size = self.0.presented().ok_or("no completed frame")?;
        let mut pixels = vec![0; size[0] as usize * size[1] as usize * 4];
        self.0.device().read(size, &mut pixels)?;
        Ok(RgbaImage::new(size[0], size[1], pixels)?)
    }
}

fn view_params(view: Viewport, source: [u32; 2], linear: bool) -> [f32; 8] {
    [
        view.left as f32,
        view.top as f32,
        view.width as f32,
        view.height as f32,
        source[0] as f32,
        source[1] as f32,
        f32::from(u8::from(linear)),
        0.0,
    ]
}

/// The software host's 11-pixel white crosshair as two solid scene quads.
fn cursor_overlays(at: [f64; 2], screen: [u32; 2]) -> Vec<[f32; BASE_FLOATS]> {
    let [x, y] = at.map(|v| v as i64);
    [[x - 5, y, 11, 1], [x, y - 5, 1, 11]]
        .map(|rect| {
            let draw = shader_draw(
                &Layer::Solid {
                    rect,
                    color: [255; 3],
                    alpha: 255,
                    blend: Blend::Replace,
                },
                screen,
            );
            let mut params = [0.0; BASE_FLOATS];
            params.copy_from_slice(&draw.params[..BASE_FLOATS]);
            params
        })
        .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_matches_the_software_crosshair() {
        let [horizontal, vertical] = cursor_overlays([100.7, 50.2], [1280, 720])[..] else {
            panic!("two quads");
        };
        // Strip corners TL, TR, BL, BR in screen pixels.
        assert_eq!(
            &horizontal[8..16],
            &[95.0, 50.0, 106.0, 50.0, 95.0, 51.0, 106.0, 51.0]
        );
        assert_eq!(
            &vertical[8..16],
            &[100.0, 45.0, 101.0, 45.0, 100.0, 56.0, 101.0, 56.0]
        );
        assert_eq!(&horizontal[..3], &[1280.0, 720.0, 255.0]);
    }
}
