//! Diagnostic: does a decide/cancel tap skip the attract opening movie?
//! Idles a fresh session until a movie player opens, then taps decide and
//! cancel and reports whether the movie stops. Read-only against the game.
use cmvs_probe::{
    checks::startup::load_script,
    device::Action,
    session::{Options, Session},
    target::Target,
};
use std::{env, fs, path::PathBuf};

fn main() {
    let mut args = env::args_os().skip(1);
    let root = PathBuf::from(args.next().expect("usage: op-skip <game-root> <fresh-user-dir>"));
    let user = PathBuf::from(args.next().expect("usage: op-skip <game-root> <fresh-user-dir>"));
    if user.exists() {
        eprintln!("user dir must not exist: {}", user.display());
        std::process::exit(2);
    }
    fs::create_dir_all(&user).unwrap();
    let target = Target::open(&root, None);
    let script = load_script(&target).expect("start script");
    let options = Options::default();
    let mut session = Session::start(&target, &script, &user, &options, None).expect("session");
    let mut opened = false;
    for i in 0..8000u64 {
        if let Err(stop) = session.frame() {
            println!("stopped before any movie after {i} frames: {stop:?}");
            return;
        }
        if session.startup().movies().has_player() {
            println!(
                "movie opened after {i} idle frames in script {}",
                session.startup().machine().current_name().escape_ascii()
            );
            opened = true;
            break;
        }
    }
    if !opened {
        println!("no movie opened within 8000 idle frames");
        return;
    }
    // Each test runs on its own fresh attract movie. Skip-hold first: the script
    // movie loop watches decide/cancel press edges, not the skip flag, so holding
    // skip (CTRL code 24) alone is expected NOT to stop the movie (the reported bug).
    for name in ["skip-hold", "decide", "cancel", "skip+confirm"] {
        let mut skipped = false;
        if name == "skip-hold" {
            session.hold(vec![24]); // CTRL = the skip/fast-forward code
            for i in 0..90u64 {
                if let Err(stop) = session.frame() {
                    println!("{name}: session stopped after {i} frames: {stop:?}");
                    return;
                }
                if !session.startup().movies().has_player() {
                    println!("{name} stopped the movie after {i} frames");
                    skipped = true;
                    break;
                }
            }
            session.hold(vec![]);
        } else if name == "skip+confirm" {
            // What the fixed Switch player produces: skip held AND a confirm
            // (decide) press injected into the same frame.
            session.hold(vec![24]);
            session.act(Action::Advance).expect("act");
            for i in 0..90u64 {
                if let Err(stop) = session.frame() {
                    println!("{name}: session stopped after {i} frames: {stop:?}");
                    return;
                }
                if !session.startup().movies().has_player() {
                    println!("{name} stopped the movie after {i} frames");
                    skipped = true;
                    break;
                }
            }
            session.hold(vec![]);
        } else {
            let action = if name == "decide" { Action::Advance } else { Action::Cancel };
            session.act(action).expect("act");
            for i in 0..90u64 {
                if let Err(stop) = session.frame() {
                    println!("{name}: session stopped after {i} frames: {stop:?}");
                    return;
                }
                if !session.startup().movies().has_player() {
                    println!("{name} stopped the movie after {i} frames");
                    skipped = true;
                    break;
                }
            }
        }
        if !skipped {
            println!("{name} did NOT stop the movie (still playing after 90 frames)");
            for i in 0..12000u64 {
                if let Err(stop) = session.frame() {
                    println!("stopped while waiting for natural end: {stop:?}");
                    return;
                }
                if !session.startup().movies().has_player() {
                    println!("movie ended naturally after {i} more frames");
                    break;
                }
            }
        }
        // Idle until the next attract movie so the next test starts the same way.
        let mut reopened = false;
        for i in 0..8000u64 {
            if let Err(stop) = session.frame() {
                println!("stopped while waiting for the next movie: {stop:?}");
                return;
            }
            if session.startup().movies().has_player() {
                println!("next movie opened after {i} idle frames");
                reopened = true;
                break;
            }
        }
        if !reopened {
            println!("no further movie opened; done");
            return;
        }
    }
}