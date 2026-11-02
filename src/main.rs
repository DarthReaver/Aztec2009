//! Aztec 2009: a terminal based clone of Autodesk Maya 2009 written in pure Rust (std only).
#![allow(dead_code)]

mod anim;
mod dag;
mod geo;
mod gui;
mod io;
mod math;
mod mel;
mod nucleus;
mod render;
mod session;
mod ui;

use session::Session;
use std::io::{BufRead, IsTerminal, Write};

const USAGE: &str = "Aztec 2009 - terminal 3D animation tool

Usage:
  aztec2009                  open the windowed workspace
  aztec2009 --tui            use the full screen terminal workspace instead
  aztec2009 --snapshot PNG [WxH]   render the perspective view to a PNG and exit
  aztec2009 -c SCRIPT        run a script (aztecsh / MEL style) and exit
  aztec2009 -e \"COMMANDS\"    run commands and exit
  aztec2009 -f SCENE.aztec   open a scene, then start the workspace
  aztec2009 --render [-s N] [-e N] [-rl LAYER] [-cam CAMERA] SCENE.aztec   command line render of the frame range (Render -s 1 -e 10)
  aztec2009 --shell          plain line based aztecsh without the full screen UI
  aztec2009 -h | --version

Inside the workspace press F1 or type help for the command list.";

fn run_text(s: &mut Session, src: &str) -> bool {
    let o = mel::execute_script(s, src);
    for l in &o.lines {
        println!("{}", l);
    }
    if let Some(e) = o.error {
        eprintln!("Error: {}", e);
        return false;
    }
    true
}

fn line_shell(s: &mut Session) {
    let stdin = std::io::stdin();
    let interactive = stdin.is_terminal();
    loop {
        if interactive {
            print!("aztecsh> ");
            let _ = std::io::stdout().flush();
        }
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let o = mel::execute_line(s, &line);
        for l in &o.lines {
            println!("{}", l);
        }
        if let Some(e) = o.error {
            eprintln!("Error: {}", e);
        }
        if s.quit {
            break;
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut session = Session::new();
    let mut i = 0;
    let mut script: Option<String> = None;
    let mut exec: Option<String> = None;
    let mut open: Option<String> = None;
    let mut force_shell = false;
    let mut force_tui = false;
    let mut snapshot: Option<(String, usize, usize)> = None;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                println!("{}", USAGE);
                return;
            }
            "--version" | "-v" => {
                println!("Aztec 2009 {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "-c" | "--script" => {
                i += 1;
                script = args.get(i).cloned();
                if script.is_none() {
                    eprintln!("-c needs a script path");
                    std::process::exit(2);
                }
            }
            "-e" | "--exec" => {
                i += 1;
                exec = args.get(i).cloned();
                if exec.is_none() {
                    eprintln!("-e needs a command string");
                    std::process::exit(2);
                }
            }
            "-f" | "--file" => {
                i += 1;
                open = args.get(i).cloned();
                if open.is_none() {
                    eprintln!("-f needs a scene path");
                    std::process::exit(2);
                }
            }
            "--render" | "-render" => {
                // Render [-s N] [-e N] [-rl LAYER] [-cam CAMERA] scene   (command line rendering)
                let mut cmd = String::from("batchRender");
                let mut scene_path: Option<String> = None;
                i += 1;
                while i < args.len() {
                    match args[i].as_str() {
                        "-s" | "-e" | "-rl" | "-cam" => {
                            let flag = match args[i].as_str() {
                                "-rl" => "-layer",
                                o => o,
                            };
                            i += 1;
                            let v = args.get(i).cloned().unwrap_or_default();
                            cmd.push_str(&format!(" {} \"{}\"", flag, v));
                        }
                        other => scene_path = Some(other.to_string()),
                    }
                    i += 1;
                }
                let Some(path) = scene_path else {
                    eprintln!("--render needs a scene file: --render [-s N] [-e N] [-rl LAYER] [-cam CAMERA] scene.aztec");
                    std::process::exit(2);
                };
                open = Some(path);
                exec = Some(cmd);
                break;
            }
            "--shell" => force_shell = true,
            "--tui" => force_tui = true,
            "--write-icon" => {
                i += 1;
                let path = args.get(i).cloned().unwrap_or_else(|| "aztec2009.png".to_string());
                let rgba = gui::png::app_icon(256);
                if let Err(e) = std::fs::write(&path, gui::png::encode_rgba(256, 256, &rgba)) {
                    eprintln!("Cannot write {}: {}", path, e);
                    std::process::exit(1);
                }
                println!("Wrote {}", path);
                return;
            }
            "--gui" => {}
            "--snapshot" => {
                i += 1;
                let path = args.get(i).cloned().unwrap_or_else(|| {
                    eprintln!("--snapshot needs an output path");
                    std::process::exit(2);
                });
                let (mut w, mut h) = (1280usize, 720usize);
                if let Some(sz) = args.get(i + 1).filter(|a| a.contains('x') && a.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)) {
                    let mut it = sz.split('x');
                    if let (Some(a), Some(b)) = (it.next().and_then(|v| v.parse().ok()), it.next().and_then(|v| v.parse().ok())) {
                        w = a;
                        h = b;
                        i += 1;
                    }
                }
                snapshot = Some((path, w, h));
            }
            other => {
                eprintln!("Unknown option '{}'\n\n{}", other, USAGE);
                std::process::exit(2);
            }
        }
        i += 1;
    }

    if let Some(path) = &open {
        let cmd = format!("file -open \"{}\"", path.replace('\\', "\\\\").replace('"', "\\\""));
        if !run_text(&mut session, &cmd) {
            std::process::exit(1);
        }
    }
    if let Some(path) = script {
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Cannot read script '{}': {}", path, e);
                std::process::exit(1);
            }
        };
        let ok = run_text(&mut session, &src);
        std::process::exit(if ok { 0 } else { 1 });
    }
    if let Some(cmds) = exec {
        let ok = run_text(&mut session, &cmds);
        std::process::exit(if ok { 0 } else { 1 });
    }
    if let Some((path, w, h)) = snapshot {
        match gui::snapshot_png(&mut session, &path, w, h) {
            Ok(_) => println!("Wrote {}", path),
            Err(e) => {
                eprintln!("Snapshot failed: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }
    let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let has_display = std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();
    if !force_tui && !force_shell && has_display {
        session.apply_startup_prefs();
        for l in mel::cmd_mel::run_user_setup(&mut session) {
            println!("{}", l);
        }
        match gui::run(session) {
            Ok(_) => return,
            Err(e) => {
                eprintln!("Cannot open the window: {}", e);
                if !tty {
                    std::process::exit(1);
                }
                eprintln!("Falling back to the terminal workspace.");
                session = Session::new();
            }
        }
    }
    if tty && !force_shell {
        match ui::run(session) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("Cannot start the full screen workspace: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }
    line_shell(&mut session);
}
