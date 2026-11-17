//! Safe ownership of the single libnx display. Calls stay on the creating thread.
#![allow(unsafe_code)]
use crate::{PadInput, Touch};
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};
static OWNED: AtomicBool = AtomicBool::new(false);

const TOUCHES: usize = 4;
/// Mirrors `CmvsNxInput` in platforms/switch/runtime/window.c.
#[repr(C)]
#[derive(Default)]
struct NxInput {
    buttons: u32,
    flags: u32,
    sticks: [[i32; 2]; 2],
    touch_count: u32,
    touches: [[u32; 3]; TOUCHES],
}
const FLAG_EXIT: u32 = 1;
const FLAG_FOCUSED: u32 = 2;
const FLAG_MODE_CHANGED: u32 = 4;

unsafe extern "C" {
    fn cmvs_nx_open(framebuffer: i32) -> i32;
    fn cmvs_nx_poll(out: *mut NxInput);
    fn cmvs_nx_present(pixels: *const u32);
    fn cmvs_nx_close();
    fn cmvs_nx_screen_size(out: *mut u32);
    fn cmvs_nx_resize(width: u32, height: u32) -> i32;
}

pub struct SwitchWindow {
    framebuffer: bool,
    screen: [u32; 2],
    mode_changed: bool,
    _thread: PhantomData<Rc<()>>,
}
impl SwitchWindow {
    /// Software presentation through [`SwitchWindow::present`]. Also locks application
    /// exit until drop, so closing from HOME is reported by [`SwitchWindow::poll`]
    /// instead of terminating the process (system timeout 15 s).
    pub fn open() -> Result<Self, &'static str> {
        Self::open_with(true)
    }
    /// Input and applet handling only, leaving the display to the deko3d compositor.
    pub fn open_gpu() -> Result<(Self, super::gpu_device::SwitchGpu), String> {
        let window = Self::open_with(false)?;
        let gpu = super::gpu_device::SwitchGpu::open()?;
        Ok((window, gpu))
    }
    fn open_with(framebuffer: bool) -> Result<Self, &'static str> {
        if OWNED
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err("Switch window already open");
        }
        // SAFETY: C rejects a second owner and cleans up partial initialization.
        if unsafe { cmvs_nx_open(i32::from(framebuffer)) } != 0 {
            OWNED.store(false, Ordering::Release);
            return Err("libnx framebuffer initialization failed");
        }
        let mut screen = [0u32; 2];
        // SAFETY: C writes exactly two u32.
        unsafe { cmvs_nx_screen_size(screen.as_mut_ptr()) };
        Ok(Self {
            framebuffer,
            screen,
            mode_changed: false,
            _thread: PhantomData,
        })
    }
    /// The current display size: 1920x1080 docked, 1280x720 handheld/tabletop.
    pub fn screen_size(&self) -> [u32; 2] {
        self.screen
    }
    /// True once after the docked/handheld operation mode changed since the previous
    /// call; the host then resizes the window and GPU to [`Self::screen_size`].
    pub fn take_mode_changed(&mut self) -> bool {
        std::mem::take(&mut self.mode_changed)
    }
    /// Rebuilds the software framebuffer for a new display size, between presents.
    /// A window opened for the GPU has no framebuffer; its swapchain is resized
    /// through `SwitchGpu::resize` instead.
    pub fn resize(&mut self, size: [u32; 2]) -> Result<(), &'static str> {
        // SAFETY: C rebuilds the framebuffer synchronously and keeps no pointer.
        if unsafe { cmvs_nx_resize(size[0], size[1]) } != 0 {
            return Err("libnx framebuffer resize failed");
        }
        self.screen = size;
        Ok(())
    }
    pub fn poll(&mut self) -> PadInput {
        let mut raw = NxInput::default();
        // SAFETY: C writes exactly one `CmvsNxInput`, a layout this struct mirrors.
        unsafe { cmvs_nx_poll(&mut raw) };
        if raw.flags & FLAG_MODE_CHANGED != 0 {
            // SAFETY: C writes exactly two u32.
            unsafe { cmvs_nx_screen_size(self.screen.as_mut_ptr()) };
            self.mode_changed = true;
        }
        let count = (raw.touch_count as usize).min(TOUCHES);
        PadInput {
            exit: raw.flags & FLAG_EXIT != 0,
            focused: raw.flags & FLAG_FOCUSED != 0,
            buttons: raw.buttons,
            sticks: raw.sticks,
            touches: raw.touches[..count]
                .iter()
                .map(|&[id, x, y]| Touch {
                    id,
                    position: [x, y],
                })
                .collect(),
        }
    }
    pub fn present(&mut self, pixels: &[u32]) -> Result<(), &'static str> {
        if !self.framebuffer {
            return Err("Switch window was opened for the GPU");
        }
        if pixels.len() as u64 != u64::from(self.screen[0]) * u64::from(self.screen[1]) {
            return Err("invalid Switch canvas length");
        }
        // SAFETY: C synchronously reads precisely screen[0]*screen[1] pixels; does not
        // retain the pointer.
        unsafe { cmvs_nx_present(pixels.as_ptr()) };
        Ok(())
    }
}
impl Drop for SwitchWindow {
    fn drop(&mut self) {
        // SAFETY: this value is the unique owner created by a successful open.
        unsafe { cmvs_nx_close() };
        OWNED.store(false, Ordering::Release);
    }
}
