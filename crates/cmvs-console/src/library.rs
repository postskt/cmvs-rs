//! Game directories on the SD card. Listing only reads directory names: identifying a
//! game hashes its scripts and executables, which waits until the player picks it.
use std::{
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

/// The previously started game, preselected in the menu.
const LAST_GAME: &str = "last-game.txt";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameEntry {
    /// Directory name, shown in the menu.
    pub name: String,
    pub root: PathBuf,
}

/// The single-game `game/` directory first, then every directory in `games/` by name.
pub fn discover(base: &Path) -> io::Result<Vec<GameEntry>> {
    let mut games = Vec::new();
    let legacy = base.join("game");
    if is_dir(&legacy)? {
        games.push(GameEntry {
            name: "game".into(),
            root: legacy,
        });
    }
    let entries = match std::fs::read_dir(base.join("games")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(games),
        Err(error) => return Err(error),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // Metadata follows links; Horizon directory entries carry no reliable type.
        if !name.starts_with('.') && is_dir(&entry.path())? {
            found.push(GameEntry {
                name,
                root: entry.path(),
            });
        }
    }
    found.sort_by_cached_key(|game| (game.name.to_lowercase(), game.name.clone()));
    games.extend(found);
    Ok(games)
}

/// Flat layout: every direct subdirectory of `base` is a game, except the player's
/// own `users/` save directory and dot directories. The legacy `game/` and `games/`
/// layouts are still recognised, so cards prepared for older players keep working.
pub fn discover_flat(base: &Path) -> io::Result<Vec<GameEntry>> {
    let mut games = Vec::new();
    let entries = match std::fs::read_dir(base) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(games),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || matches!(name.as_str(), "users" | "game" | "games") {
            continue;
        }
        // Metadata follows links; Horizon directory entries carry no reliable type.
        if is_dir(&entry.path())? {
            games.push(GameEntry {
                name,
                root: entry.path(),
            });
        }
    }
    for legacy in discover(base)? {
        if !games.iter().any(|game| game.root == legacy.root) {
            games.push(legacy);
        }
    }
    games.sort_by_cached_key(|game| (game.name.to_lowercase(), game.name.clone()));
    Ok(games)
}

fn is_dir(path: &Path) -> io::Result<bool> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(metadata.is_dir()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Index of the previously started game, or 0.
pub fn last_started(base: &Path, games: &[GameEntry]) -> usize {
    std::fs::read_to_string(base.join(LAST_GAME))
        .ok()
        .and_then(|name| games.iter().position(|g| g.name == name.trim_end()))
        .unwrap_or(0)
}

/// Best effort: a failed write only loses the preselection.
pub fn remember(base: &Path, game: &GameEntry) {
    let _ = std::fs::write(base.join(LAST_GAME), &game.name);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cmvs-console-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lists_the_legacy_directory_then_sorted_game_directories() {
        let base = scratch("library");
        assert!(discover(&base).unwrap().is_empty());
        for dir in [
            "game",
            "games/beta",
            "games/Alpha",
            "games/.hidden",
            "games/ゲーム",
        ] {
            std::fs::create_dir_all(base.join(dir)).unwrap();
        }
        std::fs::write(base.join("games/notes.txt"), "not a game").unwrap();
        let names: Vec<_> = discover(&base)
            .unwrap()
            .into_iter()
            .map(|g| g.name)
            .collect();
        assert_eq!(names, ["game", "Alpha", "beta", "ゲーム"]);

        let games = discover(&base).unwrap();
        assert_eq!(games[2].root, base.join("games/beta"));
        assert_eq!(last_started(&base, &games), 0);
        remember(&base, &games[2]);
        assert_eq!(last_started(&base, &games), 2);
        std::fs::remove_dir_all(base.join("games/beta")).unwrap();
        assert_eq!(last_started(&base, &discover(&base).unwrap()), 0);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn flat_layout_lists_game_directories_including_cjk_names() {
        let base = scratch("library-flat");
        assert!(discover_flat(&base).unwrap().is_empty());
        for dir in ["アマツツミ", "中文游戏", "Alpha", "users", ".hidden"] {
            std::fs::create_dir_all(base.join(dir)).unwrap();
        }
        std::fs::write(base.join("cmvs-switch.nro"), "program").unwrap();
        std::fs::write(base.join("font.ttf"), "font").unwrap();
        let games = discover_flat(&base).unwrap();
        let names: Vec<_> = games.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "アマツツミ", "中文游戏"]);
        assert_eq!(games[1].root, base.join("アマツツミ"));
        // Save data lives beside the games but is never listed as one.
        std::fs::create_dir_all(base.join("users/amatsutsumi")).unwrap();
        assert_eq!(discover_flat(&base).unwrap().len(), 3);
        // Save writes and the remembered game name survive CJK paths (the SD card
        // filesystem layer receives the same UTF-8 paths on Horizon).
        let user = base.join("users").join("アマツツミ");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("save.dat"), [1u8, 2, 3]).unwrap();
        assert_eq!(std::fs::read(user.join("save.dat")).unwrap(), vec![1, 2, 3]);
        remember(&base, &games[1]);
        assert_eq!(last_started(&base, &discover_flat(&base).unwrap()), 1);
        // The legacy layout keeps working on the same card.
        std::fs::create_dir_all(base.join("games/beta")).unwrap();
        let names: Vec<_> = discover_flat(&base)
            .unwrap()
            .into_iter()
            .map(|g| g.name)
            .collect();
        assert_eq!(names, ["Alpha", "beta", "アマツツミ", "中文游戏"]);
        std::fs::remove_dir_all(&base).unwrap();
    }
}
