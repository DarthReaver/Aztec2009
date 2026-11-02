//! Wavefront OBJ / MTL reader and writer.

use super::{IoKind, IoMaterial, IoNode, IoScene};
use crate::geo::Mesh;
use crate::math::{Vec2, Vec3};
use std::collections::HashMap;
use std::path::Path;

// ====================================================================== reader

struct Builder {
    name: String,
    material: Option<usize>,
    face_mat: Vec<Option<usize>>,
    mesh: Mesh,
    remap: HashMap<(usize, usize, usize), u32>,
}

impl Builder {
    fn new(name: &str, material: Option<usize>) -> Builder {
        Builder { name: name.to_string(), material, face_mat: Vec::new(), mesh: Mesh::new(), remap: HashMap::new() }
    }
}

fn resolve(idx: i64, len: usize) -> Option<usize> {
    if idx > 0 {
        let i = idx as usize - 1;
        (i < len).then_some(i)
    } else if idx < 0 {
        let back = (-idx) as usize;
        (back <= len).then(|| len - back)
    } else {
        None
    }
}

fn nums(parts: &[&str]) -> Vec<f32> {
    parts.iter().map_while(|p| p.parse::<f32>().ok()).collect()
}

pub fn read(path: &str) -> Result<IoScene, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Cannot read '{}': {}", path, e))?;
    let text = String::from_utf8_lossy(&bytes);
    let dir = Path::new(path).parent().map(|p| p.to_path_buf()).unwrap_or_default();
    parse(&text, &dir)
}

pub fn parse(text: &str, dir: &Path) -> Result<IoScene, String> {
    let mut io = IoScene { fps: 24.0, ..Default::default() };
    let mut mat_by_name: HashMap<String, usize> = HashMap::new();
    let mut pos: Vec<Vec3> = Vec::new();
    let mut uv: Vec<Vec2> = Vec::new();
    let mut nrm: Vec<Vec3> = Vec::new();
    let mut cur_name = String::from("obj");
    let mut cur_mat: Option<usize> = None;
    let mut b = Builder::new(&cur_name, None);
    let mut done: Vec<Builder> = Vec::new();

    fn flush(b: &mut Builder, done: &mut Vec<Builder>, name: &str, mat: Option<usize>) {
        let fresh = Builder::new(name, mat);
        let old = std::mem::replace(b, fresh);
        if !old.mesh.faces.is_empty() {
            done.push(old);
        }
    }

    let mut logical = String::new();
    for raw in text.lines() {
        // line continuation with a trailing backslash
        let raw = raw.trim_end();
        if let Some(stripped) = raw.strip_suffix('\\') {
            logical.push_str(stripped);
            logical.push(' ');
            continue;
        }
        logical.push_str(raw);
        let line = std::mem::take(&mut logical);
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        let key = it.next().unwrap_or("");
        let rest: Vec<&str> = it.collect();
        match key {
            "v" => {
                let n = nums(&rest);
                if n.len() >= 3 {
                    pos.push(Vec3::new(n[0], n[1], n[2]));
                }
            }
            "vt" => {
                let n = nums(&rest);
                if !n.is_empty() {
                    uv.push(Vec2::new(n[0], n.get(1).copied().unwrap_or(0.0)));
                }
            }
            "vn" => {
                let n = nums(&rest);
                if n.len() >= 3 {
                    nrm.push(Vec3::new(n[0], n[1], n[2]));
                }
            }
            "o" | "g" => {
                let name = if rest.is_empty() { "obj".to_string() } else { rest.join("_") };
                if key == "o" || b.mesh.faces.is_empty() || name != cur_name {
                    cur_name = name;
                    flush(&mut b, &mut done, &cur_name, cur_mat);
                }
            }
            "mtllib" => {
                for f in &rest {
                    if let Ok(t) = std::fs::read_to_string(dir.join(f)) {
                        parse_mtl(&t, dir, &mut io.materials, &mut mat_by_name);
                    }
                }
            }
            "usemtl" => {
                let m = rest.first().and_then(|n| mat_by_name.get(*n).copied());
                if m != cur_mat {
                    cur_mat = m;
                    if b.mesh.faces.is_empty() {
                        b.material = m;
                    }
                }
            }
            "f" => {
                let mut corner: Vec<u32> = Vec::with_capacity(rest.len());
                for tok in &rest {
                    let mut p = tok.split('/');
                    let vi = p.next().and_then(|s| s.parse::<i64>().ok()).and_then(|i| resolve(i, pos.len()));
                    let ti = p.next().and_then(|s| s.parse::<i64>().ok()).and_then(|i| resolve(i, uv.len()));
                    let ni = p.next().and_then(|s| s.parse::<i64>().ok()).and_then(|i| resolve(i, nrm.len()));
                    let Some(vi) = vi else { continue };
                    let key = (vi, ti.map(|t| t + 1).unwrap_or(0), ni.map(|n| n + 1).unwrap_or(0));
                    let idx = match b.remap.get(&key) {
                        Some(i) => *i,
                        None => {
                            let i = b.mesh.add_vertex(pos[vi], ni.map(|n| nrm[n]).unwrap_or(Vec3::UP), ti.map(|t| uv[t]).unwrap_or(Vec2::new(0.0, 0.0)));
                            b.remap.insert(key, i);
                            i
                        }
                    };
                    corner.push(idx);
                }
                if corner.len() >= 3 {
                    b.mesh.add_face(&corner);
                    b.face_mat.push(cur_mat);
                }
            }
            _ => {}
        }
    }
    flush(&mut b, &mut done, "obj", None);
    if done.is_empty() {
        return Err("The OBJ file contains no polygon faces".to_string());
    }
    for d in done {
        let mut n = IoNode::new(&d.name, None, IoKind::Mesh);
        n.mesh = d.mesh;
        n.material = d.material;
        // one material for the whole object needs no per-face list
        if d.face_mat.iter().any(|m| *m != d.material) {
            n.face_material = d.face_mat;
        }
        io.nodes.push(n);
    }
    Ok(io)
}

fn parse_mtl(text: &str, dir: &Path, mats: &mut Vec<IoMaterial>, by_name: &mut HashMap<String, usize>) {
    let mut cur: Option<usize> = None;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        let mut it = line.split_whitespace();
        let Some(key) = it.next() else { continue };
        let rest: Vec<&str> = it.collect();
        match key {
            "newmtl" => {
                let name = rest.join("_");
                mats.push(IoMaterial { name: name.clone(), color: [0.8, 0.8, 0.8], texture: None });
                cur = Some(mats.len() - 1);
                by_name.insert(name, mats.len() - 1);
            }
            "Kd" => {
                let n = nums(&rest);
                if let (Some(c), true) = (cur, n.len() >= 3) {
                    mats[c].color = [n[0], n[1], n[2]];
                }
            }
            "map_Kd" => {
                // the file name is the last token; options (-s, -o ...) come before it
                if let (Some(c), Some(f)) = (cur, rest.last()) {
                    let p = dir.join(f.replace('\\', "/"));
                    mats[c].texture = Some(p.to_string_lossy().to_string());
                }
            }
            _ => {}
        }
    }
}

// ====================================================================== writer

pub fn write(io: &IoScene, path: &str) -> Result<(), String> {
    let (obj, mtl) = to_text(io, path);
    std::fs::write(path, obj).map_err(|e| format!("Cannot write '{}': {}", path, e))?;
    if let Some(m) = mtl {
        let mp = Path::new(path).with_extension("mtl");
        std::fs::write(&mp, m).map_err(|e| format!("Cannot write '{}': {}", mp.display(), e))?;
    }
    Ok(())
}

fn mat_name(n: &str) -> String {
    super::sanitize_name(n)
}

/// OBJ has no hierarchy: every mesh is written in world space as its own `o` object.
pub fn to_text(io: &IoScene, path: &str) -> (String, Option<String>) {
    let world = io.world_matrices();
    let mut o = String::new();
    o.push_str("# Aztec 2009 OBJ export\n");
    let mtl_file = Path::new(path).with_extension("mtl").file_name().map(|f| f.to_string_lossy().to_string());
    let has_mats = io.nodes.iter().any(|n| (n.material.is_some() || n.face_material.iter().any(|m| m.is_some())) && n.kind == IoKind::Mesh);
    if let (true, Some(f)) = (has_mats, &mtl_file) {
        o.push_str(&format!("mtllib {}\n", f));
    }
    let (mut vo, mut to, mut no) = (1usize, 1usize, 1usize);
    for (i, n) in io.nodes.iter().enumerate() {
        if n.kind != IoKind::Mesh || n.mesh.faces.is_empty() {
            continue;
        }
        let m = world[i];
        let nm = m.inverse().transpose();
        o.push_str(&format!("o {}\n", super::sanitize_name(&n.name)));
        // the mesh is split per shading corner: write each distinct position, uv and normal once
        fn pool<T: Copy>(src: &[T], key: impl Fn(&T) -> [u32; 3], out: &mut Vec<T>) -> Vec<usize> {
            let mut seen: HashMap<[u32; 3], usize> = HashMap::new();
            src.iter()
                .map(|v| {
                    *seen.entry(key(v)).or_insert_with(|| {
                        out.push(*v);
                        out.len() - 1
                    })
                })
                .collect()
        }
        let (mut up, mut ut, mut un) = (Vec::new(), Vec::new(), Vec::new());
        let wp: Vec<Vec3> = n.mesh.positions.iter().map(|p| m.transform_point(*p)).collect();
        let wn: Vec<Vec3> = n.mesh.normals.iter().map(|v| nm.transform_dir(*v).normalized()).collect();
        let pi = pool(&wp, |p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()], &mut up);
        let ti = pool(&n.mesh.uvs, |t| [t.x.to_bits(), t.y.to_bits(), 0], &mut ut);
        let ni = pool(&wn, |v| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()], &mut un);
        for w in &up {
            o.push_str(&format!("v {} {} {}\n", w.x, w.y, w.z));
        }
        for t in &ut {
            o.push_str(&format!("vt {} {}\n", t.x, t.y));
        }
        for w in &un {
            o.push_str(&format!("vn {} {} {}\n", w.x, w.y, w.z));
        }
        // a mirrored transform flips the winding
        let flip = {
            let c = |k: usize| Vec3::new(m.m[k][0], m.m[k][1], m.m[k][2]);
            c(0).dot(c(1).cross(c(2))) < 0.0
        };
        let mut last_mat: Option<usize> = None;
        for (fi, f) in n.mesh.faces.iter().enumerate() {
            let fm = n.face_material.get(fi).copied().flatten().or(n.material);
            if fm != last_mat || fi == 0 {
                if let Some(mi) = fm {
                    o.push_str(&format!("usemtl {}\n", mat_name(&io.materials[mi].name)));
                }
                last_mat = fm;
            }
            o.push('f');
            let it: Box<dyn Iterator<Item = &u32>> = if flip { Box::new(f.iter().rev()) } else { Box::new(f.iter()) };
            for &v in it {
                let v = v as usize;
                o.push_str(&format!(" {}/{}/{}", pi[v] + vo, ti[v] + to, ni[v] + no));
            }
            o.push('\n');
        }
        vo += up.len();
        to += ut.len();
        no += un.len();
    }
    let mtl = has_mats.then(|| {
        let mut s = String::from("# Aztec 2009 MTL export\n");
        for m in &io.materials {
            s.push_str(&format!("newmtl {}\nKa {} {} {}\nKd {} {} {}\nKs 0.3 0.3 0.3\nNs 30\nillum 2\n", mat_name(&m.name), m.color[0] * 0.2, m.color[1] * 0.2, m.color[2] * 0.2, m.color[0], m.color[1], m.color[2]));
            if let Some(t) = &m.texture {
                s.push_str(&format!("map_Kd {}\n", t));
            }
            s.push('\n');
        }
        s
    });
    (o, mtl)
}
