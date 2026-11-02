//! Commands for scripting and customizing the program: expressions, script search path, custom shelf buttons and marking menus.

use super::commands::Out;
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::session::Session;

const EXPR_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["name", "n"], arity: 1 },
    FlagSpec { names: &["string", "s"], arity: 1 },
    FlagSpec { names: &["object", "o"], arity: 1 },
    FlagSpec { names: &["alwaysEvaluate", "ae"], arity: 1 },
    FlagSpec { names: &["unitConversion", "uc"], arity: 1 },
    FlagSpec { names: &["edit", "e"], arity: 0 },
    FlagSpec { names: &["query", "q"], arity: 0 },
    FlagSpec { names: &["delete"], arity: 0 },
    FlagSpec { names: &["list", "l"], arity: 0 },
    FlagSpec { names: &["enable"], arity: 1 },
];

/// `expression -n NAME -s "eyeball.sy = lookAt.blink" [-o OBJECT]` creates an expression; `expression -e -s "text" NAME` edits it;
/// `-q -s NAME` returns its text; `-delete NAME`; `-l` lists. Expressions run after every frame and every attribute change.
pub fn expression(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, EXPR_SPEC)?;
    let args = p.arg_strings();
    if p.has("list") {
        return Ok(s.scene.expressions.iter().map(|e| format!("{}{}{}", e.name, if e.enabled { "" } else { " (off)" }, e.error.as_ref().map(|m| format!("  // {}", m)).unwrap_or_default())).collect());
    }
    let name = p.string("name").or_else(|| args.first().cloned());
    if p.has("delete") {
        s.scene.delete_expression(&name.ok_or("Usage: expression -delete NAME")?)?;
        return Ok(vec![]);
    }
    if p.has("query") {
        // `-q -s NAME`: the -s flag swallowed the name
        let n = name.or_else(|| p.string("string")).ok_or("Usage: expression -q -s NAME")?;
        let e = s.scene.expressions.iter().find(|e| e.name == n).ok_or_else(|| format!("No expression named {}", n))?;
        return Ok(vec![e.text.clone()]);
    }
    if p.has("edit") {
        let n = name.ok_or("Usage: expression -e -s TEXT NAME")?;
        if let Some(t) = p.string("string") {
            s.scene.edit_expression(&n, &t)?;
        }
        if let Some(v) = p.num("enable")? {
            let e = s.scene.expressions.iter_mut().find(|e| e.name == n).ok_or_else(|| format!("No expression named {}", n))?;
            e.enabled = v > 0.5;
        }
        s.scene.evaluate_frame();
        return Ok(vec![]);
    }
    let text = p.string("string").ok_or("Usage: expression -n NAME -s \"object.attribute = ...\"")?;
    let n = s.scene.add_expression(p.string("name").as_deref(), &text, &p.string("object").unwrap_or_default())?;
    Ok(vec![n])
}

/// Folders searched for scripts, most specific first.
pub fn script_dirs(s: &Session) -> Vec<std::path::PathBuf> {
    let mut v: Vec<std::path::PathBuf> = Vec::new();
    if let Some(p) = &s.scene.project_dir {
        v.push(p.join("scripts"));
    }
    v.extend(s.script_dirs.iter().cloned());
    if let Some(base) = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))) {
        v.push(base.join("aztec2009").join("scripts"));
    }
    v.push(std::path::PathBuf::from("scripts"));
    if let Some(extra) = std::env::var_os("AZTEC_SCRIPT_PATH") {
        v.extend(std::env::split_paths(&extra));
    }
    v
}

/// `name.mel` (or the name as given) in the script folders.
pub fn find_script(s: &Session, name: &str) -> Option<std::path::PathBuf> {
    let file = if name.ends_with(".mel") { name.to_string() } else { format!("{}.mel", name) };
    if file.contains('/') || file.contains('\\') {
        let p = std::path::PathBuf::from(&file);
        return p.exists().then_some(p);
    }
    script_dirs(s).into_iter().map(|d| d.join(&file)).find(|p| p.is_file())
}

/// Load the script that has the name of a command that is not known yet. True when a procedure of that name exists afterwards.
pub fn autoload(s: &mut Session, name: &str) -> bool {
    if s.source_depth > 4 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return false;
    }
    let Some(path) = find_script(s, name) else { return false };
    let Ok(src) = std::fs::read_to_string(&path) else { return false };
    s.source_depth += 1;
    let o = super::commands::execute_script(s, &src);
    s.source_depth -= 1;
    o.error.is_none() && s.procs.contains_key(name)
}

/// userSetup.mel of the script folders runs when the program starts.
pub fn run_user_setup(s: &mut Session) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(p) = find_script(s, "userSetup") {
        if let Ok(src) = std::fs::read_to_string(&p) {
            let o = super::commands::execute_script(s, &src);
            out.extend(o.lines);
            if let Some(e) = o.error {
                out.push(format!("// Error in {}: {}", p.display(), e));
            }
        }
    }
    out
}

/// `scriptPath [-add DIR] [-q]`.
pub fn script_path_cmd(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["add"], arity: 1 }, FlagSpec { names: &["query", "q"], arity: 0 }])?;
    if let Some(d) = p.string("add") {
        s.script_dirs.push(std::path::PathBuf::from(d));
    }
    Ok(script_dirs(s).iter().map(|d| format!("{}{}", d.display(), if d.is_dir() { "" } else { "  (not there)" })).collect())
}

/// `cameraBookmark -add NAME | -go NAME | -delete NAME | -list`: View > Bookmarks. A bookmark keeps where the view looks from (and which camera).
pub fn camera_bookmark(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["add"], arity: 1 }, FlagSpec { names: &["go"], arity: 1 }, FlagSpec { names: &["delete"], arity: 1 }, FlagSpec { names: &["list", "l"], arity: 0 }])?;
    if let Some(n) = p.string("add") {
        let view = s.view.clone();
        match s.bookmarks.iter_mut().find(|b| b.0 == n) {
            Some(b) => b.1 = view,
            None => s.bookmarks.push((n.clone(), view)),
        }
        return Ok(vec![n]);
    }
    if let Some(n) = p.string("go") {
        let v = s.bookmarks.iter().find(|b| b.0 == n).map(|b| b.1.clone()).ok_or_else(|| format!("No bookmark named {}", n))?;
        let (mode, grid) = (s.view.mode, s.view.grid);
        s.view = v;
        s.view.mode = mode;
        s.view.grid = grid;
        s.view.apply(&mut s.scene);
        return Ok(vec![]);
    }
    if let Some(n) = p.string("delete") {
        let k = s.bookmarks.len();
        s.bookmarks.retain(|b| b.0 != n);
        if s.bookmarks.len() == k {
            return Err(format!("No bookmark named {}", n));
        }
        return Ok(vec![]);
    }
    Ok(s.bookmarks.iter().map(|b| b.0.clone()).collect())
}
