//! Semantic pad actions (save / load / config / quick save / quick load) over the
//! raw button bits, before [`cmvs_console::controls::Controls`] sees them.
//!
//! A button that belongs to any binding is held back for [`CHORD_MS`] after its
//! press: if the partner buttons of a combo arrive within that window the combo
//! fires and the whole chord is swallowed; otherwise a single-button binding
//! fires, or the button goes through to the engine map unchanged (so L still
//! opens the backlog and ZL/ZR still skip). Buttons without a binding pass
//! through with no delay. Consumed buttons stay swallowed until released, so a
//! held chord never repeats and never leaks into the game.

/// What a fired binding asks the player to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Open the game's own save screen (toolbar click).
    Save,
    /// Open the game's own load screen (toolbar click).
    Load,
    /// Open the game's own system settings (toolbar click).
    Config,
    /// Host-level snapshot write, no script menu involved.
    QSave,
    /// Host-level snapshot restore.
    QLoad,
}

/// One binding: a mask of one or two buttons and the action it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub buttons: u32,
    pub action: Action,
}

/// The solo delay for buttons that belong to a binding. Long enough to press a
/// chord comfortably, short enough that a backlog or skip still feels instant.
pub const CHORD_MS: u32 = 120;

#[derive(Debug, Default)]
pub struct Hotkeys {
    bindings: Vec<Binding>,
    /// Union of every bound button.
    members: u32,
    /// Presses held back while their chord window runs.
    held_back: u32,
    /// Per-bit elapsed hold-back time.
    delay: [u32; 16],
    /// Buttons swallowed until release (fired solos and chords).
    consumed: u32,
    previous: u32,
}

impl Hotkeys {
    pub fn new(bindings: Vec<Binding>) -> Self {
        let members = bindings.iter().fold(0u32, |mask, b| mask | b.buttons);
        Self {
            bindings,
            members,
            ..Self::default()
        }
    }

    /// Resynchronises after the player took over the input (toolbar automation,
    /// a quick load): whatever is still held is ignored until released.
    pub fn reset(&mut self, buttons: u32) {
        self.previous = buttons;
        self.consumed = buttons;
        self.held_back = 0;
        self.delay = [0; 16];
    }

    /// Filters one poll. Returns the buttons the engine map should see and the
    /// actions fired on this poll (at most one in practice).
    pub fn poll(&mut self, buttons: u32, dt_ms: u32) -> (u32, Vec<Action>) {
        let pressed = buttons & !self.previous;
        let mut fired = Vec::new();
        // Combos first: a complete chord with a fresh press fires at once, even
        // if one member was still inside its solo window.
        for binding in &self.bindings {
            let mask = binding.buttons;
            if mask.count_ones() < 2 {
                continue;
            }
            if buttons & mask == mask && pressed & mask != 0 && mask & self.consumed == 0 {
                fired.push(binding.action);
                self.consumed |= mask;
                self.held_back &= !mask;
            }
        }
        // New presses of bound buttons wait for their chord window.
        let waiting = pressed & self.members & !self.consumed;
        self.held_back |= waiting;
        for bit in 0..16 {
            if waiting & (1 << bit) != 0 {
                self.delay[bit] = 0;
            }
        }
        // Expired windows: a solo binding fires, anything else goes through.
        let mut expired = 0u32;
        for bit in 0..16 {
            let mask = 1u32 << bit;
            if self.held_back & mask == 0 {
                continue;
            }
            self.delay[bit] = self.delay[bit].saturating_add(dt_ms);
            if self.delay[bit] >= CHORD_MS {
                expired |= mask;
            }
        }
        for binding in &self.bindings {
            let mask = binding.buttons;
            if mask.count_ones() == 1 && expired & mask != 0 && mask & self.consumed == 0 {
                fired.push(binding.action);
                self.consumed |= mask;
            }
        }
        self.held_back &= !expired;
        self.consumed &= buttons;
        self.previous = buttons;
        (buttons & !self.held_back & !self.consumed, fired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmvs_native::switch_button as button;

    fn defaults() -> Hotkeys {
        Hotkeys::new(vec![
            Binding { buttons: button::X, action: Action::Save },
            Binding { buttons: button::Y, action: Action::Load },
            Binding { buttons: button::ZL | button::ZR, action: Action::Config },
            Binding { buttons: button::L | button::X, action: Action::QSave },
            Binding { buttons: button::L | button::Y, action: Action::QLoad },
        ])
    }

    #[test]
    fn unbound_buttons_pass_through_untouched() {
        let mut keys = defaults();
        let (out, fired) = keys.poll(button::A | button::B, 16);
        assert_eq!(out, button::A | button::B);
        assert!(fired.is_empty());
    }

    #[test]
    fn a_chord_fires_once_and_stays_swallowed_until_release() {
        let mut keys = defaults();
        // L presses first and waits; X completes the chord inside the window.
        let (out, fired) = keys.poll(button::L, 16);
        assert_eq!(out, 0, "L waits for its chord window");
        assert!(fired.is_empty());
        let (out, fired) = keys.poll(button::L | button::X, 16);
        assert_eq!(fired, vec![Action::QSave]);
        assert_eq!(out, 0);
        // Holding the chord does not repeat it.
        for _ in 0..10 {
            let (out, fired) = keys.poll(button::L | button::X, 16);
            assert!(fired.is_empty());
            assert_eq!(out, 0);
        }
        // Releasing X while L stays held must not open the backlog either.
        let (out, fired) = keys.poll(button::L, 16);
        assert!(fired.is_empty());
        assert_eq!(out, 0, "L was consumed by the chord");
        let (out, fired) = keys.poll(0, 16);
        assert!(fired.is_empty());
        assert_eq!(out, 0);
        // A fresh L press now works as the backlog again after its window.
        let (_, fired) = keys.poll(button::L, 16);
        assert!(fired.is_empty());
        let (out, fired) = keys.poll(button::L, CHORD_MS);
        assert!(fired.is_empty());
        assert_eq!(out, button::L);
    }

    #[test]
    fn a_solo_binding_fires_after_the_window_and_not_again_while_held() {
        let mut keys = defaults();
        let (out, fired) = keys.poll(button::X, 16);
        assert!(fired.is_empty());
        assert_eq!(out, 0, "X waits in case L follows");
        let (out, fired) = keys.poll(button::X, CHORD_MS);
        assert_eq!(fired, vec![Action::Save]);
        assert_eq!(out, 0, "the engine never sees the consumed X");
        let (out, fired) = keys.poll(button::X, 16);
        assert!(fired.is_empty());
        assert_eq!(out, 0);
        let (_, _) = keys.poll(0, 16);
        let (_, fired) = keys.poll(button::X, CHORD_MS + 1);
        assert_eq!(fired, vec![Action::Save]);
    }

    #[test]
    fn both_triggers_open_the_config_without_skipping() {
        let mut keys = defaults();
        let (out, fired) = keys.poll(button::ZL, 16);
        assert_eq!(out, 0);
        assert!(fired.is_empty());
        let (out, fired) = keys.poll(button::ZL | button::ZR, 16);
        assert_eq!(fired, vec![Action::Config]);
        assert_eq!(out, 0);
    }

    #[test]
    fn a_solo_trigger_still_skips_after_the_window() {
        let mut keys = defaults();
        keys.poll(button::ZR, 16);
        let (out, fired) = keys.poll(button::ZR, CHORD_MS);
        assert!(fired.is_empty());
        assert_eq!(out, button::ZR, "skip goes through once no chord came");
    }

    #[test]
    fn a_consumed_member_blocks_a_second_chord() {
        let mut keys = defaults();
        // X alone fires Save; pressing L afterwards must not also quick-save.
        keys.poll(button::X, 16);
        let (_, fired) = keys.poll(button::X, CHORD_MS);
        assert_eq!(fired, vec![Action::Save]);
        let (_, fired) = keys.poll(button::X | button::L, 16);
        assert!(fired.is_empty(), "X was already consumed by its solo");
    }

    #[test]
    fn reset_swallows_held_buttons_until_release() {
        let mut keys = defaults();
        keys.reset(button::L | button::A);
        let (out, fired) = keys.poll(button::L | button::A, 16);
        assert_eq!(out, 0);
        assert!(fired.is_empty());
        let (out, fired) = keys.poll(0, 16);
        assert_eq!(out, 0);
        assert!(fired.is_empty());
        let (out, fired) = keys.poll(button::A, 16);
        assert_eq!(out, button::A, "an unbound button works again after release");
        assert!(fired.is_empty());
    }
}