//! Per-game pad configuration. `buttons.txt` in the game directory overrides the
//! universal default map; without it the defaults apply. Lines are `NAME=VALUE`,
//! `#` starts a comment, names are case-insensitive.
//!
//! Pad buttons send engine input codes here; which game action a code triggers
//! still comes from the game's own `key.cfg`, which the runner reads separately.
//! Example for a game whose confirm is right click:
//!
//! ```text
//! A=MOUSE_RIGHT
//! B=MOUSE_LEFT
//! WHEEL_UP=L
//! RIGHT_STICK=SCROLL
//! ```
//!
//! The semantic actions `SAVE`, `LOAD`, `CONFIG`, `QSAVE`, `QLOAD` take one button
//! or a two-button chord (`L+X`), or `NONE`. SAVE/LOAD/CONFIG click the game's own
//! toolbar buttons (indices below); QSAVE/QLOAD use the runner's snapshot API with
//! `QSAVE_SLOT` (default 860, the slot Amatsutsumi's own quick save uses).
use crate::hotkeys::{Action, Binding};
use cmvs_console::{
    ButtonMap,
    controls::{self, RightStick},
};
use cmvs_native::switch_button as button;
use std::{io::ErrorKind, path::Path};

/// Which script window and buttons the SAVE/LOAD/CONFIG clicks target. The
/// defaults are Amatsutsumi's toolbar: window 7, buttons 0/1/2 = system/save/load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Toolbar {
    pub window: usize,
    pub save: usize,
    pub load: usize,
    pub config: usize,
}

impl Default for Toolbar {
    fn default() -> Self {
        Self { window: 7, save: 1, load: 2, config: 0 }
    }
}

/// The pad map, right-stick policy and semantic actions a game runs with.
#[derive(Debug)]
pub struct PadConfig {
    pub map: ButtonMap,
    pub right_stick: RightStick,
    pub hotkeys: Vec<Binding>,
    pub qsave_slot: u16,
    pub toolbar: Toolbar,
}

/// The universal semantic defaults: X/Y open the save/load screens (their old
/// Esc/Enter codes duplicated B/A), both triggers open the settings, and L+X /
/// L+Y quick-save/load through the runner.
fn default_hotkeys() -> Vec<Binding> {
    vec![
        Binding { buttons: button::X, action: Action::Save },
        Binding { buttons: button::Y, action: Action::Load },
        Binding { buttons: button::ZL | button::ZR, action: Action::Config },
        Binding { buttons: button::L | button::X, action: Action::QSave },
        Binding { buttons: button::L | button::Y, action: Action::QLoad },
    ]
}

/// Reads `root/buttons.txt` over the defaults. A missing file keeps the defaults;
/// a malformed line is an error naming the line, not a silent fallback.
pub fn load(root: &Path, default: ButtonMap, right_stick: RightStick) -> Result<PadConfig, String> {
    let mut config = PadConfig {
        map: default,
        right_stick,
        hotkeys: Vec::new(),
        qsave_slot: 860,
        toolbar: Toolbar::default(),
    };
    for binding in default_hotkeys() {
        set_hotkey(&mut config, binding.action, binding.buttons);
    }
    let text = match std::fs::read_to_string(root.join("buttons.txt")) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(config),
        Err(error) => return Err(format!("cannot read buttons.txt: {error}")),
    };
    for (number, line) in text.lines().enumerate() {
        let line = line.trim_start_matches('\u{feff}');
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("buttons.txt line {}: expected NAME=VALUE", number + 1))?;
        let key = key.trim().to_ascii_uppercase();
        let value = value.trim().to_ascii_uppercase();
        apply(&mut config, &key, &value)
            .map_err(|error| format!("buttons.txt line {}: {error}", number + 1))?;
    }
    Ok(config)
}

/// Replaces one semantic action. A solo binding also takes the button away from
/// the engine code map and the wheels (its old Esc/Enter/wheel job would fight
/// the action); chord members keep their solo engine job, the chord window
/// decides which one fires.
fn set_hotkey(config: &mut PadConfig, action: Action, buttons: u32) {
    config.hotkeys.retain(|b| b.action != action);
    if buttons == 0 {
        return;
    }
    config.hotkeys.push(Binding { buttons, action });
    if buttons.count_ones() == 1 {
        config.map.codes.retain(|&(mask, _)| mask != buttons);
        if config.map.wheel_up == buttons {
            config.map.wheel_up = 0;
        }
        if config.map.wheel_down == buttons {
            config.map.wheel_down = 0;
        }
    }
}

fn apply(config: &mut PadConfig, key: &str, value: &str) -> Result<(), String> {
    let map = &mut config.map;
    if key == "RIGHT_STICK" {
        config.right_stick = match value {
            "CURSOR" => RightStick::Cursor,
            "SCROLL" => RightStick::Scroll,
            _ => return Err(format!("RIGHT_STICK must be CURSOR or SCROLL, not {value}")),
        };
        return Ok(());
    }
    if let Some(action) = semantic_action(key) {
        let buttons = chord_mask(value)?;
        set_hotkey(config, action, buttons);
        return Ok(());
    }
    if key == "QSAVE_SLOT" {
        let slot: u32 = value
            .parse()
            .map_err(|_| format!("QSAVE_SLOT needs a number 0..999, not {value}"))?;
        if slot > 999 {
            return Err(format!("QSAVE_SLOT must be 0..999, not {value}"));
        }
        config.qsave_slot = slot as u16;
        return Ok(());
    }
    if let Some(slot) = toolbar_slot(key) {
        let index: usize = value
            .parse()
            .map_err(|_| format!("{key} needs a number, not {value}"))?;
        let limit = if key == "TOOLBAR_WINDOW" { 8 } else { 32 };
        if index >= limit {
            return Err(format!("{key} must be below {limit}, not {value}"));
        }
        match slot {
            0 => config.toolbar.window = index,
            1 => config.toolbar.save = index,
            2 => config.toolbar.load = index,
            _ => config.toolbar.config = index,
        }
        return Ok(());
    }
    if let Some(mask) = button_mask(key) {
        // A pad button sends one engine code (or none).
        let code = engine_code(value)?;
        map.codes.retain(|&(button, _)| button != mask);
        if let Some(code) = code {
            map.codes.push((mask, code));
        }
        return Ok(());
    }
    let slot = match key {
        "WHEEL_UP" => &mut map.wheel_up,
        "WHEEL_DOWN" => &mut map.wheel_down,
        "CONFIRM" => &mut map.confirm,
        "EXIT" => &mut map.exit,
        "MENU_UP" => &mut map.up,
        "MENU_DOWN" => &mut map.down,
        "PAGE_UP" => &mut map.page_up,
        "PAGE_DOWN" => &mut map.page_down,
        _ => return Err(format!("unknown setting {key}")),
    };
    *slot = if value == "NONE" {
        0
    } else {
        button_mask(value).ok_or_else(|| format!("{key} needs a button name, not {value}"))?
    };
    Ok(())
}

fn semantic_action(name: &str) -> Option<Action> {
    Some(match name {
        "SAVE" => Action::Save,
        "LOAD" => Action::Load,
        "CONFIG" => Action::Config,
        "QSAVE" => Action::QSave,
        "QLOAD" => Action::QLoad,
        _ => return None,
    })
}

fn toolbar_slot(name: &str) -> Option<u8> {
    Some(match name {
        "TOOLBAR_WINDOW" => 0,
        "TOOLBAR_SAVE" => 1,
        "TOOLBAR_LOAD" => 2,
        "TOOLBAR_CONFIG" => 3,
        _ => return None,
    })
}

/// `NONE` removes the action; otherwise one button or a `BTN1+BTN2` chord.
fn chord_mask(value: &str) -> Result<u32, String> {
    if value == "NONE" {
        return Ok(0);
    }
    let parts: Vec<&str> = value.split('+').map(str::trim).collect();
    if parts.is_empty() || parts.len() > 2 {
        return Err(format!("expected one button or BTN1+BTN2, not {value}"));
    }
    let mut mask = 0u32;
    for part in parts {
        let bit = button_mask(part).ok_or_else(|| format!("unknown button {part}"))?;
        if mask & bit != 0 {
            return Err(format!("button {part} appears twice in {value}"));
        }
        mask |= bit;
    }
    Ok(mask)
}

fn button_mask(name: &str) -> Option<u32> {
    Some(match name {
        "A" => button::A,
        "B" => button::B,
        "X" => button::X,
        "Y" => button::Y,
        "L" => button::L,
        "R" => button::R,
        "ZL" => button::ZL,
        "ZR" => button::ZR,
        "PLUS" => button::PLUS,
        "MINUS" => button::MINUS,
        "UP" => button::UP,
        "DOWN" => button::DOWN,
        "LEFT" => button::LEFT,
        "RIGHT" => button::RIGHT,
        "STICKL" => button::STICK_L,
        "STICKR" => button::STICK_R,
        _ => return None,
    })
}

/// `NONE` removes a binding; every other name is an engine input code.
fn engine_code(name: &str) -> Result<Option<usize>, String> {
    Ok(Some(match name {
        "NONE" => return Ok(None),
        "MOUSE_LEFT" => controls::MOUSE_LEFT,
        "MOUSE_RIGHT" => controls::MOUSE_RIGHT,
        "WHEEL_UP" => controls::WHEEL_UP,
        "WHEEL_DOWN" => controls::WHEEL_DOWN,
        "ENTER" => controls::ENTER,
        "SHIFT" => controls::SHIFT,
        "CTRL" => controls::CTRL,
        "ESCAPE" => controls::ESCAPE,
        "ARROW_UP" => controls::ARROW_UP,
        "ARROW_DOWN" => controls::ARROW_DOWN,
        "ARROW_LEFT" => controls::ARROW_LEFT,
        "ARROW_RIGHT" => controls::ARROW_RIGHT,
        _ => return Err(format!("unknown code {name}")),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cmvs-buttons-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn defaults() -> ButtonMap {
        ButtonMap {
            confirm: button::A,
            exit: button::PLUS,
            up: button::UP,
            down: button::DOWN,
            page_up: button::L,
            page_down: button::R,
            codes: vec![
                (button::A, controls::MOUSE_LEFT),
                (button::X, controls::ESCAPE),
                (button::Y, controls::ENTER),
            ],
            wheel_up: button::L,
            wheel_down: button::MINUS,
        }
    }

    #[test]
    fn missing_file_keeps_defaults() {
        let dir = scratch("missing");
        let config = load(&dir, defaults(), RightStick::Cursor).unwrap();
        assert_eq!(config.map.codes, vec![(button::A, controls::MOUSE_LEFT)]);
        assert_eq!(config.right_stick, RightStick::Cursor);
        assert_eq!(config.qsave_slot, 860);
        assert_eq!(config.toolbar, Toolbar::default());
        let hotkeys = &config.hotkeys;
        assert!(hotkeys.contains(&Binding { buttons: button::X, action: Action::Save }));
        assert!(hotkeys.contains(&Binding { buttons: button::Y, action: Action::Load }));
        assert!(
            hotkeys.contains(&Binding { buttons: button::ZL | button::ZR, action: Action::Config })
        );
        assert!(hotkeys.contains(&Binding { buttons: button::L | button::X, action: Action::QSave }));
        assert!(hotkeys.contains(&Binding { buttons: button::L | button::Y, action: Action::QLoad }));
        // The default X/Y solos removed their redundant Esc/Enter engine codes.
        assert!(!config.map.codes.iter().any(|&(mask, _)| mask & (button::X | button::Y) != 0));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn overrides_codes_wheels_and_stick_policy() {
        let dir = scratch("override");
        std::fs::write(
            dir.join("buttons.txt"),
            "\u{feff}# 中文注释\r\na=MOUSE_RIGHT\nB = NONE\nWHEEL_DOWN=R\nRIGHT_STICK=scroll\nEXIT=MINUS\n",
        )
        .unwrap();
        let config = load(&dir, defaults(), RightStick::Cursor).unwrap();
        assert_eq!(
            config.map.codes,
            vec![(button::A, controls::MOUSE_RIGHT)],
            "A replaced, B removed"
        );
        assert_eq!(config.map.wheel_down, button::R);
        assert_eq!(config.map.exit, button::MINUS);
        assert_eq!(config.right_stick, RightStick::Scroll);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn malformed_lines_name_the_line() {
        let dir = scratch("bad");
        std::fs::write(dir.join("buttons.txt"), "A=MOUSE_LEFT\nWHAT=ENTER\n").unwrap();
        let error = load(&dir, defaults(), RightStick::Cursor).unwrap_err();
        assert!(error.contains("line 2"), "{error}");
        std::fs::write(dir.join("buttons.txt"), "A=TELEPORT\n").unwrap();
        let error = load(&dir, defaults(), RightStick::Cursor).unwrap_err();
        assert!(error.contains("line 1") && error.contains("TELEPORT"), "{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hotkeys_accept_solos_chords_and_none() {
        let dir = scratch("hotkeys");
        std::fs::write(
            dir.join("buttons.txt"),
            "SAVE=MINUS\nCONFIG=L+R\nQSAVE=NONE\n",
        )
        .unwrap();
        let config = load(&dir, defaults(), RightStick::Cursor).unwrap();
        assert!(config.hotkeys.contains(&Binding { buttons: button::MINUS, action: Action::Save }));
        assert!(config.hotkeys.contains(&Binding { buttons: button::L | button::R, action: Action::Config }));
        assert!(!config.hotkeys.iter().any(|b| b.action == Action::QSave), "NONE removes");
        // The rest of the defaults survive.
        assert!(config.hotkeys.contains(&Binding { buttons: button::Y, action: Action::Load }));
        assert!(config.hotkeys.contains(&Binding { buttons: button::L | button::Y, action: Action::QLoad }));
        // The MINUS solo took the wheel-down slot away from it.
        assert_eq!(config.map.wheel_down, 0);
        // Chord members keep their solo jobs: L still scrolls the backlog.
        assert_eq!(config.map.wheel_up, button::L);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn qsave_slot_and_toolbar_indices_are_checked() {
        let dir = scratch("slots");
        std::fs::write(
            dir.join("buttons.txt"),
            "QSAVE_SLOT=100\nTOOLBAR_WINDOW=5\nTOOLBAR_SAVE=3\nTOOLBAR_LOAD=4\nTOOLBAR_CONFIG=6\n",
        )
        .unwrap();
        let config = load(&dir, defaults(), RightStick::Cursor).unwrap();
        assert_eq!(config.qsave_slot, 100);
        assert_eq!(
            config.toolbar,
            Toolbar { window: 5, save: 3, load: 4, config: 6 }
        );
        std::fs::write(dir.join("buttons.txt"), "QSAVE_SLOT=1000\n").unwrap();
        let error = load(&dir, defaults(), RightStick::Cursor).unwrap_err();
        assert!(error.contains("line 1") && error.contains("1000"), "{error}");
        std::fs::write(dir.join("buttons.txt"), "TOOLBAR_WINDOW=8\n").unwrap();
        assert!(load(&dir, defaults(), RightStick::Cursor).is_err());
        std::fs::write(dir.join("buttons.txt"), "TOOLBAR_SAVE=32\n").unwrap();
        assert!(load(&dir, defaults(), RightStick::Cursor).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn malformed_chords_are_rejected() {
        let dir = scratch("chords");
        for (text, needle) in [
            ("SAVE=L+X+Y", "BTN1+BTN2"),
            ("SAVE=L+L", "twice"),
            ("SAVE=L+WHAT", "WHAT"),
        ] {
            std::fs::write(dir.join("buttons.txt"), text).unwrap();
            let error = load(&dir, defaults(), RightStick::Cursor).unwrap_err();
            assert!(error.contains("line 1") && error.contains(needle), "{error}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}