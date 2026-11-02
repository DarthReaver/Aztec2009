//! Raw terminal handling with zero crates: termios via libc symbols that std already links,
//! an input thread, and a parser for keys and SGR mouse reports.

use std::io::{Read, Write};
use std::os::raw::c_ulong;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Mutex;

#[repr(C)]
#[derive(Clone, Copy)]
struct Termios {
    c_iflag: u32,
    c_oflag: u32,
    c_cflag: u32,
    c_lflag: u32,
    c_line: u8,
    c_cc: [u8; 32],
    c_ispeed: u32,
    c_ospeed: u32,
}

#[repr(C)]
#[derive(Default)]
struct Winsize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

extern "C" {
    fn tcgetattr(fd: i32, t: *mut Termios) -> i32;
    fn tcsetattr(fd: i32, actions: i32, t: *const Termios) -> i32;
    fn cfmakeraw(t: *mut Termios);
    fn ioctl(fd: i32, request: c_ulong, ...) -> i32;
}

const TCSANOW: i32 = 0;
const TIOCGWINSZ: c_ulong = 0x5413;

static ORIGINAL: Mutex<Option<Termios>> = Mutex::new(None);

const ENTER_SEQ: &str = "\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1002h\x1b[?1006h\x1b[2J";
const LEAVE_SEQ: &str = "\x1b[?1006l\x1b[?1002l\x1b[?1000l\x1b[?25h\x1b[0m\x1b[?1049l";

/// Restore the terminal to its original state. Safe to call more than once and from a panic hook.
pub fn restore_terminal() {
    let mut out = std::io::stdout();
    let _ = out.write_all(LEAVE_SEQ.as_bytes());
    let _ = out.flush();
    if let Ok(mut g) = ORIGINAL.lock() {
        if let Some(t) = g.take() {
            unsafe {
                tcsetattr(0, TCSANOW, &t);
            }
        }
    }
}

pub struct RawTerm;

impl RawTerm {
    pub fn enter() -> std::io::Result<RawTerm> {
        let mut t = std::mem::MaybeUninit::<Termios>::uninit();
        let rc = unsafe { tcgetattr(0, t.as_mut_ptr()) };
        if rc != 0 {
            return Err(std::io::Error::other("stdin is not a terminal"));
        }
        let orig = unsafe { t.assume_init() };
        let mut raw = orig;
        unsafe {
            cfmakeraw(&mut raw);
        }
        // Blocking reads of at least one byte.
        raw.c_cc[6] = 1; // VMIN
        raw.c_cc[5] = 0; // VTIME
        if unsafe { tcsetattr(0, TCSANOW, &raw) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if let Ok(mut g) = ORIGINAL.lock() {
            *g = Some(orig);
        }
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_terminal();
            prev_hook(info);
        }));
        let mut out = std::io::stdout();
        out.write_all(ENTER_SEQ.as_bytes())?;
        out.flush()?;
        Ok(RawTerm)
    }
}

impl Drop for RawTerm {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Terminal size in character cells (columns, rows).
pub fn term_size() -> (usize, usize) {
    let mut ws = Winsize::default();
    let rc = unsafe { ioctl(1, TIOCGWINSZ, &mut ws as *mut Winsize) };
    if rc == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
        (ws.ws_col as usize, ws.ws_row as usize)
    } else {
        (80, 24)
    }
}

pub fn supports_truecolor() -> bool {
    let ct = std::env::var("COLORTERM").unwrap_or_default().to_ascii_lowercase();
    if ct.contains("truecolor") || ct.contains("24bit") {
        return true;
    }
    let term = std::env::var("TERM").unwrap_or_default();
    // Most modern emulators handle truecolor even when they do not advertise it.
    !(term == "linux" || term == "vt100" || term == "dumb" || term == "ansi")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Esc,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
    Ctrl(char),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MouseKind {
    Down(Button),
    Up(Button),
    Drag(Button),
    WheelUp,
    WheelDown,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MouseEvent {
    pub kind: MouseKind,
    pub x: usize,
    pub y: usize,
    pub mods: Mods,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Key(Key, Mods),
    Mouse(MouseEvent),
}

enum Parsed {
    Event(Event, usize),
    Skip(usize),
    Incomplete,
}

fn mods_from_param(p: u32) -> Mods {
    let m = p.saturating_sub(1);
    Mods { shift: m & 1 != 0, alt: m & 2 != 0, ctrl: m & 4 != 0 }
}

fn f_key_from_tilde(n: u32) -> Option<u8> {
    Some(match n {
        11 => 1,
        12 => 2,
        13 => 3,
        14 => 4,
        15 => 5,
        17 => 6,
        18 => 7,
        19 => 8,
        20 => 9,
        21 => 10,
        23 => 11,
        24 => 12,
        _ => return None,
    })
}

fn parse_csi(buf: &[u8]) -> Parsed {
    // buf starts after "ESC [".
    let mut end = None;
    for (i, &b) in buf.iter().enumerate() {
        if (0x40..=0x7e).contains(&b) {
            end = Some(i);
            break;
        }
        if i > 40 {
            return Parsed::Skip(buf.len().min(41) + 2);
        }
    }
    let end = match end {
        Some(e) => e,
        None => return Parsed::Incomplete,
    };
    let consumed = end + 3;
    let final_byte = buf[end];
    let params = std::str::from_utf8(&buf[..end]).unwrap_or("");

    if let Some(rest) = params.strip_prefix('<') {
        let nums: Vec<usize> = rest.split(';').filter_map(|p| p.parse().ok()).collect();
        if nums.len() == 3 && (final_byte == b'M' || final_byte == b'm') {
            let b = nums[0];
            let mods = Mods { shift: b & 4 != 0, alt: b & 8 != 0, ctrl: b & 16 != 0 };
            let x = nums[1].saturating_sub(1);
            let y = nums[2].saturating_sub(1);
            let kind = if b & 64 != 0 {
                if b & 3 == 0 {
                    MouseKind::WheelUp
                } else {
                    MouseKind::WheelDown
                }
            } else {
                let btn = match b & 3 {
                    0 => Button::Left,
                    1 => Button::Middle,
                    _ => Button::Right,
                };
                if final_byte == b'm' {
                    MouseKind::Up(btn)
                } else if b & 32 != 0 {
                    MouseKind::Drag(btn)
                } else {
                    MouseKind::Down(btn)
                }
            };
            return Parsed::Event(Event::Mouse(MouseEvent { kind, x, y, mods }), consumed);
        }
        return Parsed::Skip(consumed);
    }

    let nums: Vec<u32> = params.split(';').filter_map(|p| p.parse().ok()).collect();
    let mods = if nums.len() >= 2 { mods_from_param(nums[1]) } else { Mods::default() };
    let key = match final_byte {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'H' => Key::Home,
        b'F' => Key::End,
        b'Z' => Key::BackTab,
        b'P' => Key::F(1),
        b'Q' => Key::F(2),
        b'R' => Key::F(3),
        b'S' => Key::F(4),
        b'~' => match nums.first().copied().unwrap_or(0) {
            1 | 7 => Key::Home,
            4 | 8 => Key::End,
            3 => Key::Delete,
            5 => Key::PageUp,
            6 => Key::PageDown,
            n => match f_key_from_tilde(n) {
                Some(f) => Key::F(f),
                None => return Parsed::Skip(consumed),
            },
        },
        _ => return Parsed::Skip(consumed),
    };
    Parsed::Event(Event::Key(key, mods), consumed)
}

fn parse_one(buf: &[u8]) -> Parsed {
    if buf.is_empty() {
        return Parsed::Incomplete;
    }
    let b0 = buf[0];
    if b0 == 0x1b {
        if buf.len() == 1 {
            return Parsed::Event(Event::Key(Key::Esc, Mods::default()), 1);
        }
        match buf[1] {
            b'[' => return parse_csi(&buf[2..]),
            b'O' => {
                if buf.len() < 3 {
                    return Parsed::Incomplete;
                }
                let key = match buf[2] {
                    b'A' => Key::Up,
                    b'B' => Key::Down,
                    b'C' => Key::Right,
                    b'D' => Key::Left,
                    b'H' => Key::Home,
                    b'F' => Key::End,
                    b'P' => Key::F(1),
                    b'Q' => Key::F(2),
                    b'R' => Key::F(3),
                    b'S' => Key::F(4),
                    _ => return Parsed::Skip(3),
                };
                return Parsed::Event(Event::Key(key, Mods::default()), 3);
            }
            0x1b => return Parsed::Event(Event::Key(Key::Esc, Mods::default()), 1),
            _ => {
                // Alt + key
                return match parse_one(&buf[1..]) {
                    Parsed::Event(Event::Key(k, mut m), n) => {
                        m.alt = true;
                        Parsed::Event(Event::Key(k, m), n + 1)
                    }
                    Parsed::Event(e, n) => Parsed::Event(e, n + 1),
                    Parsed::Skip(n) => Parsed::Skip(n + 1),
                    Parsed::Incomplete => Parsed::Incomplete,
                };
            }
        }
    }
    let ev = |k: Key| Parsed::Event(Event::Key(k, Mods::default()), 1);
    match b0 {
        0x0d | 0x0a => ev(Key::Enter),
        0x09 => ev(Key::Tab),
        0x7f | 0x08 => ev(Key::Backspace),
        0x01..=0x1a => ev(Key::Ctrl((b'a' + b0 - 1) as char)),
        0x00 | 0x1c..=0x1f => Parsed::Skip(1),
        _ => {
            let len = if b0 < 0x80 {
                1
            } else if b0 >> 5 == 0b110 {
                2
            } else if b0 >> 4 == 0b1110 {
                3
            } else if b0 >> 3 == 0b11110 {
                4
            } else {
                return Parsed::Skip(1);
            };
            if buf.len() < len {
                return Parsed::Incomplete;
            }
            match std::str::from_utf8(&buf[..len]).ok().and_then(|s| s.chars().next()) {
                Some(c) => {
                    let shift = c.is_uppercase();
                    Parsed::Event(Event::Key(Key::Char(c), Mods { shift, alt: false, ctrl: false }), len)
                }
                None => Parsed::Skip(len),
            }
        }
    }
}

/// Parse as many complete events as possible from `pending`, leaving any partial sequence behind.
pub fn parse_events(pending: &mut Vec<u8>) -> Vec<Event> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < pending.len() {
        match parse_one(&pending[pos..]) {
            Parsed::Event(e, n) => {
                out.push(e);
                pos += n;
            }
            Parsed::Skip(n) => pos += n.max(1),
            Parsed::Incomplete => break,
        }
    }
    pending.drain(..pos.min(pending.len()));
    out
}

/// Start a thread that reads stdin and forwards parsed events. The channel closes when stdin does.
pub fn spawn_input() -> Receiver<Event> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 512];
        let mut pending: Vec<u8> = Vec::new();
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    pending.extend_from_slice(&buf[..n]);
                    // A trailing lone ESC that never completes is treated as the Esc key by parse_one.
                    for e in parse_events(&mut pending) {
                        if tx.send(e).is_err() {
                            return;
                        }
                    }
                    if pending.len() > 4096 {
                        pending.clear();
                    }
                }
            }
        }
    });
    rx
}
