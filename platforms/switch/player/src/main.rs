mod audio;
mod backend;
mod boost;
mod buttons;
mod gpu;
mod hotkeys;
mod toast;
#[cfg(target_os = "horizon")]
mod sys {
    pub use cmvs_native::{
        SwitchAudioOut, SwitchGpu, SwitchWindow, prefer_core, set_cpu_boost, shared_font,
        show_system_error, software_keyboard, total_memory,
    };
}
#[cfg(not(target_os = "horizon"))]
#[path = "stub.rs"]
mod sys;

use cmvs_console::{
    ButtonMap, Console, Labels,
    controls::{self, Controls},
    encoding::{self, SystemFont},
    error_screen, library, menu,
};
use cmvs_core::{
    input::KeyBindings,
    profile::Registry,
    raster::RgbaImage,
    runner::{Startup, StartupEvent, load_start},
    viewport::{Viewport, presentation_mode},
};
use cmvs_font::font::FontRenderer;
use cmvs_native::switch_button as button;
use std::{
    error::Error,
    io::{ErrorKind, Read},
    path::Path,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const BASE: &str = "sdmc:/switch/cmvs";
const LOG_PATH: &str = "sdmc:/switch/cmvs/error.log";
/// `LOG_PATH` as the player sees it on the SD card.
const LOG: &str = "switch/cmvs/error.log";
/// VM budget per loop iteration before polling input and the applet again.
const BUDGET: Duration = Duration::from_millis(8);
const IDLE: Duration = Duration::from_millis(16);

/// The combined standard controller. In game, A clicks at the pointer, B is right
/// click (cancel / hide the message window in most games), R and ZR are Ctrl and ZL
/// Shift (skip), L sends wheel-up (the backlog in Amatsutsumi) and - wheel-down; the
/// D-pad uses keyboard navigation and the left stick moves the pointer. A game's
/// `buttons.txt` overrides the in-game part; `key.cfg` still decides what each code
/// does inside the game.
fn console(screen: [u32; 2]) -> Console {
    Console {
        screen,
        buttons: ButtonMap {
            confirm: button::A,
            exit: button::PLUS,
            up: button::UP,
            down: button::DOWN,
            page_up: button::L,
            page_down: button::R,
            codes: vec![
                (button::A, controls::MOUSE_LEFT),
                (button::B, controls::MOUSE_RIGHT),
                (button::X, controls::ESCAPE),
                (button::Y, controls::ENTER),
                (button::ZL, controls::SHIFT),
                (button::ZR, controls::CTRL),
                (button::R, controls::CTRL),
                (button::UP, controls::ARROW_UP),
                (button::DOWN, controls::ARROW_DOWN),
                (button::LEFT, controls::ARROW_LEFT),
                (button::RIGHT, controls::ARROW_RIGHT),
            ],
            wheel_up: button::L,
            wheel_down: button::MINUS,
        },
        labels: Labels {
            confirm: "A",
            exit: "+",
            pages: "L / R",
        },
    }
}

/// Decoded images kept for reloads; skip mode and scene changes load the same sprites
/// repeatedly. An application has over 3 GiB, where high-resolution games (about
/// 15 MiB a sprite) keep reloading with less than this; an applet has a few hundred
/// MiB in all and keeps the size of the GPU texture cache.
fn decoded_cache_bytes(total_memory: u64) -> usize {
    if total_memory >= 2 << 30 {
        512 << 20
    } else {
        128 << 20
    }
}

fn millis(duration: Duration) -> u32 {
    duration.as_millis().min(u128::from(u32::MAX)) as u32
}

/// `font.ttf` beside the program wins, then one packed into an NSP's RomFS,
/// otherwise the system's shared Japanese font.
fn font_bytes(base: &Path) -> Result<Vec<u8>> {
    for dir in [base, Path::new("romfs:/")] {
        match std::fs::read(dir.join("font.ttf")) {
            Ok(bytes) => return Ok(bytes),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(sys::shared_font(false).ok_or("no font.ttf and no system shared font")?)
}

/// `font.ttf` beside the program or in the NSP's RomFS alone wins; otherwise the
/// shared fonts the game's text needs, the first available one primary and the rest
/// as fallbacks.
fn game_font(base: &Path, encoding: cmvs_core::TextEncoding) -> Result<FontRenderer> {
    let renderer = |bytes| -> Result<FontRenderer> {
        Ok(FontRenderer::from_bytes(bytes, 0)?.with_text_encoding(encoding))
    };
    for dir in [base, Path::new("romfs:/")] {
        match std::fs::read(dir.join("font.ttf")) {
            Ok(bytes) => return renderer(bytes),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let mut fonts = encoding::system_fonts(encoding)
        .iter()
        .filter_map(|&font| sys::shared_font(font == SystemFont::SimplifiedChinese));
    let font = renderer(
        fonts
            .next()
            .ok_or("no font.ttf and no system shared font")?,
    )?;
    for bytes in fonts {
        font.add_fallback(bytes, 0)?;
    }
    Ok(font)
}

/// How a game session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ended {
    /// The game quit, or the player held + to leave it.
    Quit,
    /// The system asked the application to close.
    Exit,
}

fn render_gpu(
    startup: &Startup<'_>,
    font: &mut FontRenderer,
    gpu: &mut gpu::GpuRenderer,
    linear: bool,
) -> Result<()> {
    let size = startup.screen_size();
    if let Some(frame) = startup.movie_frame() {
        return gpu.render_image(frame, size, linear);
    }
    let objects = font.prepare_objects(startup.messages())?;
    let mut extra: Vec<_> = objects.iter().collect();
    extra.extend(startup.messages().visible_window_objects());
    let frame = cmvs_core::scene_render::prepare_scene(
        startup.scene(),
        Some(startup.camera()),
        &extra,
        size[0],
        size[1],
    )?;
    gpu.render(&frame)
}

fn render(startup: &Startup<'_>, font: &mut FontRenderer) -> Result<RgbaImage> {
    if let Some(frame) = startup.movie_frame() {
        return Ok(frame.clone());
    }
    let objects = font.prepare_objects(startup.messages())?;
    let mut extra: Vec<_> = objects.iter().collect();
    extra.extend(startup.messages().visible_window_objects());
    let size = startup.screen_size();
    Ok(cmvs_core::scene_render::render_scene(
        startup.scene(),
        Some(startup.camera()),
        &extra,
        size[0],
        size[1],
    )?)
}

/// Host-only crosshair; the engine screenshot readback never includes it.
fn draw_cursor(surface: &mut [u32], screen: [u32; 2], view: Viewport, pointer: [i32; 2]) {
    let at = view.to_target(pointer);
    for d in -5..=5 {
        for (x, y) in [
            (at[0] as i32 + d, at[1] as i32),
            (at[0] as i32, at[1] as i32 + d),
        ] {
            if (0..screen[0] as i32).contains(&x) && (0..screen[1] as i32).contains(&y) {
                surface[y as usize * screen[0] as usize + x as usize] = 0xffffff;
            }
        }
    }
}

/// A synthetic toolbar click, replicating the upstream menu tests: eight engine
/// frames with the pointer on the button and mouse-left held for three of them
/// (the frames after steps 3, 4 and 5). The player advances one step per engine
/// input boundary, so the click lands at the same cadence as a real button press.
struct AutoClick {
    target: [i32; 2],
    button: u32,
    step: u32,
    buttons: u32,
}

impl AutoClick {
    fn new(target: [i32; 2], button: u32) -> Self {
        Self { target, button, step: 0, buttons: 0 }
    }

    /// Advances after an input boundary; `false` once the whole click is done.
    fn advance(&mut self) -> bool {
        self.step += 1;
        self.buttons = if (3..6).contains(&self.step) { self.button } else { 0 };
        self.step < 8
    }
}

/// The canvas-space centre of a script toolbar button, when the toolbar window is
/// actually up and the button is clickable. Save screens and the title menu have
/// no such toolbar; the caller reports that instead of clicking blind.
fn toolbar_target(
    startup: &Startup<'_>,
    toolbar: &buttons::Toolbar,
    index: usize,
) -> Option<[i32; 2]> {
    let window = startup.messages().window(toolbar.window)?;
    if !window.visible || window.hidden {
        return None;
    }
    let button = window.buttons.get(index)?;
    let [x, y, w, h] = button.hit.map(i32::from);
    if !button.enabled || w == 0 || h == 0 {
        return None;
    }
    Some([
        window.position[0] + x + w / 2,
        window.position[1] + y + h / 2,
    ])
}

fn run(
    window: &mut sys::SwitchWindow,
    gpu: Option<&mut gpu::GpuRenderer>,
    root: &Path,
    host_font: &ab_glyph::FontVec,
) -> Result<Ended> {
    // Boosted through game loading; normal clocks return when this drops.
    let mut load = boost::LoadBoost::starting();
    let mut cpu_boost = boost::CpuBoost::default();
    cpu_boost.apply(load.wanted(true));
    let base = Path::new(BASE);
    let game = Registry::builtin().identify_game(root, None)?;
    let record = game.selection.record.ok_or("unregistered game")?;
    let script = load_start(root, &game.files)?;
    let encoding = cmvs_console::encoding::select(root, &game.files, &script)?;
    let mut startup = Startup::new_with_encoding(&script, root, &game, 0, encoding)?;
    let user = base.join("users").join(&record.game);
    startup.set_user_data_directory(&user)?;
    startup.set_block_unreviewed(false);
    startup.set_block_presumed(false);
    let (audio, audio_thread) = audio::AudioThread::start(root)?;
    let audio_control = audio.host_control();
    startup.set_audio_backend(Box::new(audio))?;
    startup.set_decode_movie_frames(true);
    // Core 0 runs the VM and rendering, core 1 the audio thread.
    startup.set_mgv_factory(Box::new(
        cmvs_video::PrefetchTheoraFactory {
            on_thread_start: || {
                sys::prefer_core(2);
            },
        }
        .with_lookahead(192 << 20, 144),
    ));
    startup.set_scene_prefetch(|| {
        sys::prefer_core(2);
    });
    // Image planes decode on this thread and on one more, on the core the movie
    // prefetch uses: the audio core stays free.
    startup.set_image_decode_threads(cmvs_core::image::DecodeThreads {
        extra: 1,
        on_start: Some(|_| {
            sys::prefer_core(2);
        }),
    });
    startup.set_decoded_image_cache(decoded_cache_bytes(sys::total_memory()));
    startup.set_record_text(true);
    // Requested screen captures read back the canvas instead of a software redraw.
    startup.set_host_capture(gpu.is_some());
    startup.set_save_clock(Box::new(cmvs_core::runner::utc_save_time));
    let mut font = game_font(base, encoding)?;
    font.set_user_directory(&user)?;
    startup.set_glyph_metrics(font.metrics());
    if !font.has_face(startup.settings().font_face()) {
        startup.set_font_face(cmvs_core::settings::DEFAULT_FONT_FACE)?;
    }
    let mut keys = String::new();
    std::fs::File::open(root.join("key.cfg"))?
        .take(65537)
        .read_to_string(&mut keys)?;
    let bindings = KeyBindings::parse(&keys)?;
    let pad = buttons::load(root, console(window.screen_size()).buttons, controls::RightStick::Cursor)?;
    let mut hotkeys = hotkeys::Hotkeys::new(pad.hotkeys.clone());
    // Toolbar automation clicks with whichever button sends mouse-left (A by default).
    let click_button = pad
        .map
        .codes
        .iter()
        .find(|&&(_, code)| code == controls::MOUSE_LEFT)
        .map_or(button::A, |&(mask, _)| mask);
    let mut auto: Option<AutoClick> = None;
    let mut toast: Option<toast::Toast> = None;
    let mut size = startup.screen_size();
    let mut backend = backend::Backend {
        display: startup.display_preferences(),
        size,
        frame: None,
        gpu,
        modal_shown: false,
        encoding,
    };
    backend.display.render_mode =
        cmvs_core::platform::supported_render_mode(&backend, backend.display.render_mode)?;

    let mut screen = window.screen_size();
    let mut surface = match backend.gpu {
        Some(_) => Vec::new(),
        None => vec![0u32; (screen[0] * screen[1]) as usize],
    };
    let mut controls = Controls::new(size, pad.map.clone()).right_stick(pad.right_stick);
    // The button that started the game must not click it.
    controls.suppress_held(startup.input_mut());
    let mut focused = true;
    let mut input_due = true;
    let mut last_poll = Instant::now();
    let mut last_frame = last_poll;
    let ended = loop {
        let mut sample = window.poll();
        let raw_buttons = sample.buttons;
        if sample.exit {
            break Ended::Exit;
        }
        if window.take_mode_changed() {
            // Docked/handheld switch: rebuild the display at the new size. The
            // script canvas is unchanged, so the viewport recentres it below.
            screen = window.screen_size();
            window.resize(screen)?;
            if let Some(gpu) = backend.gpu.as_mut() {
                gpu.resize(screen)?;
            } else {
                surface = vec![0u32; (screen[0] * screen[1]) as usize];
            }
        }
        audio_thread.check()?;
        let now = Instant::now();
        let dt = millis(now.duration_since(last_poll));
        last_poll = now;
        if sample.focused != focused {
            focused = sample.focused;
            audio_control.set_active(focused);
            startup.set_focused(focused);
            controls.focus(focused, startup.input_mut());
            last_frame = now;
        }
        cpu_boost.apply(load.wanted(focused));
        if toast.as_ref().is_some_and(|t| !t.alive()) {
            toast = None;
        }
        if let Some(click) = &auto {
            // The synthetic click owns the pad until it finishes; the real HOME
            // and focus flags still apply.
            sample.buttons = click.buttons;
            sample.sticks = [[0, 0]; 2];
            sample.touches.clear();
        } else {
            let (effective, fired) = hotkeys.poll(sample.buttons, dt);
            sample.buttons = effective;
            for action in fired {
                match action {
                    hotkeys::Action::QSave => {
                        let slot = pad.qsave_slot;
                        let text = match startup.save_slot(slot) {
                            Ok(true) => format!("Quick save OK [{slot}]"),
                            Ok(false) => "Quick save unavailable here".to_owned(),
                            Err(error) => {
                                report(&format!("quick save slot {slot}: {error}"));
                                "Quick save failed (see error.log)".to_owned()
                            }
                        };
                        toast = Some(toast::Toast::new(host_font, &text, screen));
                    }
                    hotkeys::Action::QLoad => {
                        let slot = pad.qsave_slot;
                        match startup.load_slot(slot, &mut backend) {
                            Ok(()) => {
                                controls.suppress_held(startup.input_mut());
                                hotkeys.reset(raw_buttons);
                                let loaded = startup.screen_size();
                                if loaded != size {
                                    size = loaded;
                                    backend.size = loaded;
                                    controls = Controls::new(size, pad.map.clone())
                                        .right_stick(pad.right_stick);
                                    controls.suppress_held(startup.input_mut());
                                }
                                toast = Some(toast::Toast::new(
                                    host_font,
                                    &format!("Quick load OK [{slot}]"),
                                    screen,
                                ));
                            }
                            Err(error) => {
                                let text = if error.to_string().contains("does not exist") {
                                    format!("No quick save in slot {slot}")
                                } else {
                                    report(&format!("quick load slot {slot}: {error}"));
                                    "Quick load failed (see error.log)".to_owned()
                                };
                                toast = Some(toast::Toast::new(host_font, &text, screen));
                            }
                        }
                    }
                    hotkeys::Action::Save | hotkeys::Action::Load | hotkeys::Action::Config => {
                        let index = match action {
                            hotkeys::Action::Save => pad.toolbar.save,
                            hotkeys::Action::Load => pad.toolbar.load,
                            _ => pad.toolbar.config,
                        };
                        match toolbar_target(&startup, &pad.toolbar, index) {
                            Some(target) => {
                                controls.warp(target);
                                auto = Some(AutoClick::new(target, click_button));
                            }
                            None => {
                                toast = Some(toast::Toast::new(
                                    host_font,
                                    "Menu unavailable here",
                                    screen,
                                ));
                            }
                        }
                    }
                }
            }
        }
        let mode = presentation_mode(
            backend.display.fullscreen,
            backend.display.aspect_scaling,
            backend.display.aspect_mode,
        );
        let view = Viewport::new(size, screen, mode)?;
        // A script movie wait loop (the opening attract movie in particular)
        // watches the decide / cancel press edges, not the skip flag, so holding
        // the skip button alone never ends it. While a full-screen movie is up,
        // turn a held skip button into a confirm press: the loop consumes it and
        // stops the movie. The injection lasts only as long as the movie does, so
        // the still-held skip button then resumes its normal fast-forward role
        // without leaking a click into the title menu.
        if auto.is_none() && startup.movies().has_player() {
            let skip_held = pad.map.codes.iter().any(|&(mask, code)| {
                (code == controls::CTRL || code == controls::SHIFT) && sample.buttons & mask != 0
            });
            if skip_held {
                sample.buttons |= pad.map.confirm;
            }
        }
        controls.poll(&sample, view, dt);
        if controls.quit_requested() {
            break Ended::Quit;
        }
        if input_due {
            if let Some(click) = &mut auto {
                // The pointer stays pinned to the button for the whole click, as
                // in the upstream tests; a script warp must not drag it away.
                startup.input_mut().take_warp();
                controls.warp(click.target);
                controls.frame(&bindings, startup.input_mut());
                if !click.advance() {
                    auto = None;
                    controls.suppress_held(startup.input_mut());
                    hotkeys.reset(raw_buttons);
                }
            } else {
                if let Some(warp) = startup.input_mut().take_warp() {
                    controls.warp(warp);
                }
                controls.frame(&bindings, startup.input_mut());
            }
            input_due = false;
        }
        let budget = Instant::now();
        while budget.elapsed() < BUDGET {
            let tick = Instant::now();
            let event = startup.tick(&mut backend).map_err(|error| {
                let vm = startup.machine();
                format!(
                    "{error}\nat {} / 0x{:x} after {} steps",
                    vm.current_name().escape_ascii(),
                    vm.pc(),
                    startup.steps()
                )
            })?;
            if std::mem::take(&mut backend.modal_shown) {
                // Time inside the keyboard is not game time, and the button that
                // closed it must not click the game.
                controls.suppress_held(startup.input_mut());
                last_frame = Instant::now();
                last_poll = last_frame;
                load.interrupted();
            } else {
                load.ran(tick.elapsed());
            }
            match event {
                StartupEvent::Progress | StartupEvent::SnapshotCaptured => {}
                StartupEvent::Quit => {
                    startup.save_preferences(backend.display)?;
                    return Ok(Ended::Quit);
                }
                StartupEvent::Inactive => {
                    load.interrupted();
                    std::thread::sleep(IDLE);
                    last_frame = Instant::now();
                    break;
                }
                StartupEvent::Yield => {
                    let now = Instant::now();
                    load.frame(now);
                    startup.advance_decoded_frame(millis(now.duration_since(last_frame)))?;
                    last_frame = now;
                    let linear = backend.display.render_mode == 1;
                    if let Some(gpu) = &mut backend.gpu {
                        render_gpu(&startup, &mut font, gpu, linear)?;
                        let cursor = controls
                            .cursor_visible()
                            .then(|| view.to_target(controls.pointer()));
                        let toast = toast
                            .as_ref()
                            .map(|t| (t.image(), toast::position(screen, t.image())));
                        gpu.present(screen, mode, linear, cursor, toast)?;
                        if startup.capture_due() {
                            startup.supply_capture(gpu.read()?)?;
                        }
                    } else {
                        let frame = render(&startup, &mut font)?;
                        surface.fill(0);
                        frame.draw_scaled(&mut surface, screen[0], screen[1], mode, linear)?;
                        if let Some(t) = &toast {
                            toast::blit(&mut surface, screen, t.image());
                        }
                        if controls.cursor_visible() {
                            draw_cursor(&mut surface, screen, view, controls.pointer());
                        }
                        window.present(&surface)?;
                        backend.frame = Some(frame);
                    }
                    input_due = true;
                    break;
                }
                event => return Err(format!("game stopped: {event:?}").into()),
            }
        }
    };
    startup.save_preferences(backend.display)?;
    Ok(ended)
}

/// Shows a full-screen host image through whichever renderer is active.
fn show(
    window: &mut sys::SwitchWindow,
    gpu: Option<&mut gpu::GpuRenderer>,
    image: &RgbaImage,
    screen: [u32; 2],
) -> Result<()> {
    if let Some(gpu) = gpu {
        gpu.render_image(image, screen, false)?;
        return gpu.present(screen, 1, false, None, None);
    }
    let mut surface = vec![0u32; (screen[0] * screen[1]) as usize];
    image.draw_scaled(&mut surface, screen[0], screen[1], 1, false)?;
    Ok(window.present(&surface)?)
}

/// Runs the selection screen; `None` when the player or the system closes it.
fn choose(
    window: &mut sys::SwitchWindow,
    mut gpu: Option<&mut gpu::GpuRenderer>,
    games: &[library::GameEntry],
    font: &ab_glyph::FontVec,
    selected: usize,
) -> Result<Option<usize>> {
    let first = window.poll();
    if first.exit {
        return Ok(None);
    }
    let mut screen = window.screen_size();
    let mut menu = menu::Menu::new(selected, games.len(), &first, &console(screen));
    let mut dirty = true;
    let mut last = Instant::now();
    loop {
        let sample = window.poll();
        if sample.exit {
            return Ok(None);
        }
        if window.take_mode_changed() {
            screen = window.screen_size();
            window.resize(screen)?;
            if let Some(gpu) = gpu.as_deref_mut() {
                gpu.resize(screen)?;
            }
            let (selected, top) = (menu.selected, menu.top);
            menu = menu::Menu::new(selected, games.len(), &sample, &console(screen));
            menu.top = top.min(games.len().saturating_sub(1));
            dirty = true;
        }
        let now = Instant::now();
        let dt = millis(now.duration_since(last));
        last = now;
        match menu.poll(&sample, dt) {
            menu::MenuAction::Idle => {}
            menu::MenuAction::Moved => dirty = true,
            menu::MenuAction::Exit => return Ok(None),
            menu::MenuAction::Launch(index) => {
                // Identification and loading take a while; say what is happening.
                show(
                    window,
                    gpu.as_deref_mut(),
                    &menu::draw(&menu, games, font, true),
                    screen,
                )?;
                return Ok(Some(index));
            }
        }
        if dirty && sample.focused {
            show(
                window,
                gpu.as_deref_mut(),
                &menu::draw(&menu, games, font, false),
                screen,
            )?;
            dirty = false;
        }
        std::thread::sleep(IDLE);
    }
}

/// Shows `message` until the player leaves the page. When the page itself cannot be
/// shown (for example after a GPU failure), the system error dialog stands in.
fn show_error(
    window: &mut sys::SwitchWindow,
    mut gpu: Option<&mut gpu::GpuRenderer>,
    font: &ab_glyph::FontVec,
    game: Option<&str>,
    message: &str,
    can_return: bool,
) -> error_screen::ErrorChoice {
    // The page titles the game; the log and the system dialog name it inline.
    let logged = match game {
        Some(game) => format!("{game}: {message}"),
        None => message.to_owned(),
    };
    report(&logged);
    let mut page = |window: &mut sys::SwitchWindow| -> Result<error_screen::ErrorChoice> {
        let first = window.poll();
        if first.exit {
            return Ok(error_screen::ErrorChoice::Exit);
        }
        let mut screen = window.screen_size();
        let ui = console(screen);
        let image = error_screen::draw(&ui, font, game, message, LOG, can_return);
        show(window, gpu.as_deref_mut(), &image, screen)?;
        let mut prompt = error_screen::ErrorPrompt::new(&first, can_return, &ui.buttons);
        loop {
            let sample = window.poll();
            if sample.exit {
                return Ok(error_screen::ErrorChoice::Exit);
            }
            if window.take_mode_changed() {
                screen = window.screen_size();
                window.resize(screen)?;
                if let Some(gpu) = gpu.as_deref_mut() {
                    gpu.resize(screen)?;
                }
                let ui = console(screen);
                let image = error_screen::draw(&ui, font, game, message, LOG, can_return);
                show(window, gpu.as_deref_mut(), &image, screen)?;
            }
            if let Some(choice) = prompt.poll(&sample) {
                return Ok(choice);
            }
            std::thread::sleep(IDLE);
        }
    };
    page(window).unwrap_or_else(|page_error| {
        system_error(&format!("{logged}\n\nThe error page failed: {page_error}"));
        error_screen::ErrorChoice::Exit
    })
}

/// Logs and shows an error through the system dialog, for failures without a page.
fn system_error(message: &str) {
    report(message);
    let summary: String = message
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect();
    sys::show_system_error(&summary, &format!("{message}\n\nSaved to {LOG}"));
}

/// Single-game NSP boot, ahead of the SD menu. Mode A: the game's own files are
/// packed at `romfs:/` (its `data/pack/` scripts identify it), so the RomFS is the
/// read-only game root. Mode B: `romfs:/target.txt` names one folder under
/// `sdmc:/switch/cmvs/` to run. Neither: the regular multi-game menu. Saves always
/// go to `sdmc:/switch/cmvs/users/<game id>/`, whichever mode booted.
fn boot_root(base: &Path) -> std::result::Result<Option<(std::path::PathBuf, String)>, String> {
    boot_from(Path::new("romfs:/"), base)
}

/// The testable core of [`boot_root`]: `romfs` stands in for the mounted RomFS.
fn boot_from(romfs: &Path, base: &Path) -> std::result::Result<Option<(std::path::PathBuf, String)>, String> {
    if cmvs_core::profile::GameFiles::discover(romfs).is_ok() {
        return Ok(Some((romfs.to_path_buf(), "romfs".to_owned())));
    }
    let text = match std::fs::read_to_string(romfs.join("target.txt")) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read romfs:/target.txt: {error}")),
    };
    let name = text.trim_start_matches('\u{feff}').trim();
    if name.is_empty() {
        return Ok(None);
    }
    if name.contains(['/', '\\']) || matches!(name, "." | "..") {
        return Err(format!("target.txt must name one folder, not {name:?}"));
    }
    let root = base.join(name);
    if !root.is_dir() {
        return Err(format!("target.txt names {name:?}, but {BASE}/{name}/ does not exist"));
    }
    Ok(Some((root, name.to_owned())))
}

/// One game starts directly; several are offered in a menu that games return to.
/// Errors are shown here; with a game list, the player may go back to it.
fn session(
    window: &mut sys::SwitchWindow,
    mut gpu: Option<&mut gpu::GpuRenderer>,
    font: &ab_glyph::FontVec,
) {
    let base = Path::new(BASE);
    // An NSP boots straight into its game; there is no menu to return to.
    match boot_root(base) {
        Ok(Some((root, name))) => {
            let result = run(window, gpu.as_deref_mut(), &root, font);
            if let Some(gpu) = gpu.as_deref_mut() {
                gpu.clear_cache();
            }
            if let Err(error) = result {
                show_error(window, gpu, font, Some(&name), &error.to_string(), false);
            }
            return;
        }
        Ok(None) => {}
        Err(error) => {
            show_error(window, gpu, font, None, &error, false);
            return;
        }
    }
    let games = match library::discover_flat(base) {
        Ok(games) if !games.is_empty() => games,
        Ok(_) => {
            let message = format!("no games found: put each game directory in {BASE}/");
            show_error(window, gpu, font, None, &message, false);
            return;
        }
        Err(error) => {
            let message = format!("cannot list games: {error}");
            show_error(window, gpu, font, None, &message, false);
            return;
        }
    };
    let single = games.len() == 1;
    let mut selected = library::last_started(base, &games);
    loop {
        if !single {
            match choose(window, gpu.as_deref_mut(), &games, font, selected) {
                Ok(Some(index)) => selected = index,
                Ok(None) => return,
                Err(error) => {
                    show_error(window, gpu, font, None, &error.to_string(), false);
                    return;
                }
            }
            library::remember(base, &games[selected]);
        }
        let game = &games[selected];
        let result = run(window, gpu.as_deref_mut(), &game.root, font);
        if let Some(gpu) = gpu.as_deref_mut() {
            gpu.clear_cache();
        }
        match result {
            Ok(Ended::Exit) => return,
            Ok(Ended::Quit) if single => return,
            Ok(Ended::Quit) => {}
            Err(error) => {
                let name = Some(game.name.as_str());
                let choice = show_error(
                    window,
                    gpu.as_deref_mut(),
                    font,
                    name,
                    &error.to_string(),
                    !single,
                );
                if choice == error_screen::ErrorChoice::Exit {
                    return;
                }
            }
        }
    }
}

fn main() {
    // A panic aborts the process; leave its message where errors are expected.
    std::panic::set_hook(Box::new(|info| report(&format!("panic: {info}"))));
    // The error log needs its directory; an NSP may boot without any SD layout.
    let _ = std::fs::create_dir_all(BASE);
    // The window lives until main returns, after the error log is written: dropping
    // it unlocks exit, after which a pending HOME close may end the process.
    let gpu = match cmvs_console::gpu::use_gpu(Path::new(BASE)) {
        Ok(gpu) => gpu,
        Err(error) => return system_error(&error.to_string()),
    };
    let opened = if gpu {
        sys::SwitchWindow::open_gpu().map(|(window, gpu)| (window, Some(gpu)))
    } else {
        sys::SwitchWindow::open()
            .map(|window| (window, None))
            .map_err(String::from)
    };
    let (mut window, gpu) = match opened {
        Ok(opened) => opened,
        Err(error) => return system_error(&error),
    };
    let mut gpu = match gpu.map(gpu::GpuRenderer::new).transpose() {
        Ok(gpu) => gpu,
        Err(error) => return system_error(&error.to_string()),
    };
    let font = match font_bytes(Path::new(BASE)).and_then(|bytes| {
        Ok(ab_glyph::FontVec::try_from_vec_and_index(bytes, 0).map_err(|_| "invalid font")?)
    }) {
        Ok(font) => font,
        Err(error) => return system_error(&format!("cannot load the font: {error}")),
    };
    session(&mut window, gpu.as_mut(), &font);
}

fn report(error: &str) {
    eprintln!("Switch player: {error}");
    let _ = std::fs::write(LOG_PATH, error);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_an_application_memory_budget_gets_the_large_decoded_cache() {
        assert_eq!(decoded_cache_bytes(0), 128 << 20);
        assert_eq!(decoded_cache_bytes(442 << 20), 128 << 20);
        assert_eq!(decoded_cache_bytes(3276 << 20), 512 << 20);
    }

    #[test]
    fn a_clicks_r_and_zr_skip_l_opens_the_backlog() {
        let console = console([1280, 720]);
        let codes = &console.buttons.codes;
        assert!(codes.contains(&(button::A, controls::MOUSE_LEFT)));
        assert!(codes.contains(&(button::B, controls::MOUSE_RIGHT)));
        assert!(codes.contains(&(button::ZR, controls::CTRL)));
        assert!(codes.contains(&(button::R, controls::CTRL)));
        assert_eq!(
            (console.buttons.wheel_up, console.buttons.wheel_down),
            (button::L, button::MINUS)
        );
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cmvs-boot-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn no_romfs_falls_back_to_the_menu() {
        let dir = scratch("none");
        let base = dir.join("base");
        std::fs::create_dir_all(&base).unwrap();
        assert_eq!(boot_from(&dir.join("romfs"), &base).unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn packed_game_files_boot_mode_a() {
        let dir = scratch("mode-a");
        let romfs = dir.join("romfs");
        std::fs::create_dir_all(romfs.join("data/pack")).unwrap();
        std::fs::write(romfs.join("data/pack/start.ps3"), [0u8; 4]).unwrap();
        std::fs::write(romfs.join("data/pack/script.cpz"), [0u8; 4]).unwrap();
        // Even with a target.txt present, the packed game wins.
        std::fs::write(romfs.join("target.txt"), "アマツツミ").unwrap();
        let base = dir.join("base");
        let (root, name) = boot_from(&romfs, &base).unwrap().expect("mode A");
        assert_eq!(root, romfs);
        assert_eq!(name, "romfs");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn target_txt_boots_the_named_sd_folder() {
        let dir = scratch("mode-b");
        let romfs = dir.join("romfs");
        let base = dir.join("base");
        std::fs::create_dir_all(romfs.join("")).unwrap();
        std::fs::create_dir_all(base.join("アマツツミ")).unwrap();
        std::fs::write(romfs.join("target.txt"), "\u{feff} アマツツミ \r\n").unwrap();
        let (root, name) = boot_from(&romfs, &base).unwrap().expect("mode B");
        assert_eq!(root, base.join("アマツツミ"));
        assert_eq!(name, "アマツツミ");
        // An empty target keeps the menu.
        std::fs::write(romfs.join("target.txt"), "  \n").unwrap();
        assert_eq!(boot_from(&romfs, &base).unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_or_escaping_target_is_an_error() {
        let dir = scratch("bad-target");
        let romfs = dir.join("romfs");
        let base = dir.join("base");
        std::fs::create_dir_all(&romfs).unwrap();
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(romfs.join("target.txt"), "not-there").unwrap();
        let error = boot_from(&romfs, &base).unwrap_err();
        assert!(error.contains("not-there"), "{error}");
        for bad in ["../escape", "a/b", ".."] {
            std::fs::write(romfs.join("target.txt"), bad).unwrap();
            assert!(boot_from(&romfs, &base).is_err(), "{bad}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
