//! FBX container layer: the generic node tree with binary (6.x / 7.x) and ASCII readers and writers.

use std::io::Read;

#[derive(Clone, Debug)]
pub enum Prop {
    Bool(bool),
    I32(i32),
    I64(i64),
    F64(f64),
    Str(String),
    Raw(Vec<u8>),
    ArrD(Vec<f64>),
    ArrF(Vec<f32>),
    ArrI(Vec<i32>),
    ArrL(Vec<i64>),
}

impl Prop {
    pub fn f64(&self) -> Option<f64> {
        match self {
            Prop::Bool(v) => Some(*v as i32 as f64),
            Prop::I32(v) => Some(*v as f64),
            Prop::I64(v) => Some(*v as f64),
            Prop::F64(v) => Some(*v),
            _ => None,
        }
    }
    pub fn i64(&self) -> Option<i64> {
        match self {
            Prop::Bool(v) => Some(*v as i64),
            Prop::I32(v) => Some(*v as i64),
            Prop::I64(v) => Some(*v),
            Prop::F64(v) => Some(*v as i64),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Prop::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn vec_f64(&self) -> Option<Vec<f64>> {
        match self {
            Prop::ArrD(v) => Some(v.clone()),
            Prop::ArrF(v) => Some(v.iter().map(|x| *x as f64).collect()),
            Prop::ArrI(v) => Some(v.iter().map(|x| *x as f64).collect()),
            Prop::ArrL(v) => Some(v.iter().map(|x| *x as f64).collect()),
            _ => None,
        }
    }
    pub fn vec_i64(&self) -> Option<Vec<i64>> {
        match self {
            Prop::ArrD(v) => Some(v.iter().map(|x| *x as i64).collect()),
            Prop::ArrF(v) => Some(v.iter().map(|x| *x as i64).collect()),
            Prop::ArrI(v) => Some(v.iter().map(|x| *x as i64).collect()),
            Prop::ArrL(v) => Some(v.clone()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct FNode {
    pub name: String,
    pub props: Vec<Prop>,
    pub children: Vec<FNode>,
}

impl FNode {
    pub fn new(name: &str) -> FNode {
        FNode { name: name.to_string(), props: Vec::new(), children: Vec::new() }
    }
    pub fn with(mut self, p: Prop) -> FNode {
        self.props.push(p);
        self
    }
    pub fn child(&self, name: &str) -> Option<&FNode> {
        self.children.iter().find(|c| c.name == name)
    }
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a FNode> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
    pub fn add(&mut self, c: FNode) -> &mut FNode {
        self.children.push(c);
        self.children.last_mut().unwrap()
    }
    /// `Child: value` shorthand.
    pub fn leaf(&mut self, name: &str, p: Prop) {
        self.children.push(FNode::new(name).with(p));
    }
    pub fn str_at(&self, i: usize) -> &str {
        self.props.get(i).and_then(|p| p.str()).unwrap_or("")
    }
}

pub const BINARY_MAGIC: &[u8] = b"Kaydara FBX Binary  \0";

pub fn is_binary(data: &[u8]) -> bool {
    data.starts_with(BINARY_MAGIC)
}

// ====================================================================== binary reader

struct Rd<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.p.checked_add(n).filter(|e| *e <= self.d.len()).ok_or("Unexpected end of FBX file")?;
        let s = &self.d[self.p..end];
        self.p = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}

fn read_array(r: &mut Rd, ty: u8) -> Result<Prop, String> {
    let n = r.u32()? as usize;
    let enc = r.u32()?;
    let clen = r.u32()? as usize;
    let size = match ty {
        b'd' | b'l' => 8,
        _ => 4,
    };
    let want = n.checked_mul(size).ok_or("Corrupt FBX array")?;
    let raw: Vec<u8> = match enc {
        0 => r.take(want)?.to_vec(),
        1 => {
            let comp = r.take(clen)?;
            let mut out = Vec::with_capacity(want);
            flate2::read::ZlibDecoder::new(comp).take(want as u64).read_to_end(&mut out).map_err(|e| format!("Corrupt compressed FBX array: {}", e))?;
            out
        }
        _ => return Err("Unknown FBX array encoding".to_string()),
    };
    if raw.len() < want {
        return Err("Truncated FBX array".to_string());
    }
    Ok(match ty {
        b'd' => Prop::ArrD(raw.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect()),
        b'f' => Prop::ArrF(raw.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()),
        b'l' => Prop::ArrL(raw.chunks_exact(8).map(|c| i64::from_le_bytes(c.try_into().unwrap())).collect()),
        b'i' => Prop::ArrI(raw.chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).collect()),
        _ => Prop::ArrI(raw.iter().map(|b| *b as i32).collect()),
    })
}

fn read_node(r: &mut Rd, wide: bool) -> Result<Option<FNode>, String> {
    let (end, nprops, _plen) = if wide { (r.u64()? as usize, r.u64()? as usize, r.u64()? as usize) } else { (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize) };
    let nlen = r.u8()? as usize;
    if end == 0 {
        return Ok(None);
    }
    let name = String::from_utf8_lossy(r.take(nlen)?).to_string();
    let mut node = FNode::new(&name);
    for _ in 0..nprops {
        let t = r.u8()?;
        let p = match t {
            b'Y' => Prop::I32(i16::from_le_bytes(r.take(2)?.try_into().unwrap()) as i32),
            b'C' => Prop::Bool(r.u8()? != 0),
            b'I' => Prop::I32(r.u32()? as i32),
            b'F' => Prop::F64(f32::from_le_bytes(r.take(4)?.try_into().unwrap()) as f64),
            b'D' => Prop::F64(f64::from_le_bytes(r.take(8)?.try_into().unwrap())),
            b'L' => Prop::I64(r.u64()? as i64),
            b'S' => {
                let n = r.u32()? as usize;
                Prop::Str(String::from_utf8_lossy(r.take(n)?).to_string())
            }
            b'R' => {
                let n = r.u32()? as usize;
                Prop::Raw(r.take(n)?.to_vec())
            }
            b'd' | b'f' | b'l' | b'i' | b'b' => read_array(r, t)?,
            _ => return Err(format!("Unknown FBX property type '{}'", t as char)),
        };
        node.props.push(p);
    }
    if r.p < end {
        while r.p < end {
            match read_node(r, wide)? {
                Some(c) => node.children.push(c),
                None => break,
            }
        }
    }
    r.p = end.min(r.d.len()).max(r.p);
    Ok(Some(node))
}

/// Returns the top level nodes and the file version (for example 7400).
pub fn parse_binary(data: &[u8]) -> Result<(Vec<FNode>, u32), String> {
    let mut r = Rd { d: data, p: BINARY_MAGIC.len() + 2 };
    let version = r.u32()?;
    let wide = version >= 7500;
    let mut out = Vec::new();
    while r.p < data.len() {
        match read_node(&mut r, wide) {
            Ok(Some(n)) => out.push(n),
            Ok(None) => break,
            Err(e) => {
                if out.is_empty() {
                    return Err(e);
                }
                break;
            }
        }
    }
    Ok((out, version))
}

// ====================================================================== binary writer

pub fn write_binary(nodes: &[FNode], version: u32) -> Vec<u8> {
    let wide = version >= 7500;
    let mut o: Vec<u8> = Vec::new();
    o.extend_from_slice(BINARY_MAGIC);
    o.extend_from_slice(&[0x1A, 0x00]);
    o.extend_from_slice(&version.to_le_bytes());
    for n in nodes {
        write_node(&mut o, n, wide);
    }
    o.extend_from_slice(&vec![0u8; if wide { 25 } else { 13 }]);
    // footer in the layout other exporters use
    o.extend_from_slice(&[0xfa, 0xbc, 0xab, 0x09, 0xd0, 0xc8, 0xd4, 0x66, 0xb1, 0x76, 0xfb, 0x83, 0x1c, 0xf7, 0x26, 0x7e]);
    let mut pad = ((o.len() + 15) & !15) - o.len();
    if pad == 0 {
        pad = 16;
    }
    o.extend_from_slice(&vec![0u8; pad]);
    o.extend_from_slice(&version.to_le_bytes());
    o.extend_from_slice(&[0u8; 120]);
    o.extend_from_slice(&[0xf8, 0x5a, 0x8c, 0x6a, 0xde, 0xf5, 0xd9, 0x7e, 0xec, 0xe9, 0x0c, 0xe3, 0x75, 0x8f, 0x29, 0x0b]);
    o
}

fn put_prop(o: &mut Vec<u8>, p: &Prop) {
    match p {
        Prop::Bool(v) => {
            o.push(b'C');
            o.push(*v as u8);
        }
        Prop::I32(v) => {
            o.push(b'I');
            o.extend_from_slice(&v.to_le_bytes());
        }
        Prop::I64(v) => {
            o.push(b'L');
            o.extend_from_slice(&v.to_le_bytes());
        }
        Prop::F64(v) => {
            o.push(b'D');
            o.extend_from_slice(&v.to_le_bytes());
        }
        Prop::Str(s) => {
            o.push(b'S');
            o.extend_from_slice(&(s.len() as u32).to_le_bytes());
            o.extend_from_slice(s.as_bytes());
        }
        Prop::Raw(b) => {
            o.push(b'R');
            o.extend_from_slice(&(b.len() as u32).to_le_bytes());
            o.extend_from_slice(b);
        }
        Prop::ArrD(v) => put_arr(o, b'd', v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect()),
        Prop::ArrF(v) => put_arr(o, b'f', v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect()),
        Prop::ArrI(v) => put_arr(o, b'i', v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect()),
        Prop::ArrL(v) => put_arr(o, b'l', v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect()),
    }
}

fn put_arr(o: &mut Vec<u8>, ty: u8, n: usize, bytes: Vec<u8>) {
    o.push(ty);
    o.extend_from_slice(&(n as u32).to_le_bytes());
    o.extend_from_slice(&0u32.to_le_bytes());
    o.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    o.extend_from_slice(&bytes);
}

fn write_node(o: &mut Vec<u8>, n: &FNode, wide: bool) {
    let start = o.len();
    let hdr = if wide { 24 } else { 12 };
    o.extend_from_slice(&vec![0u8; hdr]);
    o.push(n.name.len().min(255) as u8);
    o.extend_from_slice(&n.name.as_bytes()[..n.name.len().min(255)]);
    let pstart = o.len();
    for p in &n.props {
        put_prop(o, p);
    }
    let plen = o.len() - pstart;
    if !n.children.is_empty() {
        for c in &n.children {
            write_node(o, c, wide);
        }
        o.extend_from_slice(&vec![0u8; if wide { 25 } else { 13 }]);
    }
    let end = o.len();
    if wide {
        o[start..start + 8].copy_from_slice(&(end as u64).to_le_bytes());
        o[start + 8..start + 16].copy_from_slice(&(n.props.len() as u64).to_le_bytes());
        o[start + 16..start + 24].copy_from_slice(&(plen as u64).to_le_bytes());
    } else {
        o[start..start + 4].copy_from_slice(&(end as u32).to_le_bytes());
        o[start + 4..start + 8].copy_from_slice(&(n.props.len() as u32).to_le_bytes());
        o[start + 8..start + 12].copy_from_slice(&(plen as u32).to_le_bytes());
    }
}

// ====================================================================== ASCII writer

fn asc_float(v: f64) -> String {
    if v.is_finite() {
        format!("{:?}", v)
    } else {
        "0".to_string()
    }
}

/// "Name\0\x01Class" (binary form) becomes "Class::Name".
fn asc_str(s: &str) -> String {
    let s = match s.split_once("\0\x01") {
        Some((a, b)) => format!("{}::{}", b, a),
        None => s.to_string(),
    };
    format!("\"{}\"", s.replace('"', "&quot;").replace('\n', "&lf;").replace('\r', "&cr;"))
}

pub fn write_ascii(nodes: &[FNode], version: u32) -> String {
    let mut o = String::new();
    o.push_str(&format!("; FBX {}.{}.0 project file\n; ------------------------------------------------\n\n", version / 1000, (version % 1000) / 100));
    for n in nodes {
        asc_node(&mut o, n, 0);
    }
    o
}

fn asc_node(o: &mut String, n: &FNode, depth: usize) {
    let tab = "\t".repeat(depth);
    let arr = n.props.len() == 1 && matches!(n.props[0], Prop::ArrD(_) | Prop::ArrF(_) | Prop::ArrI(_) | Prop::ArrL(_));
    if arr {
        let (len, body): (usize, String) = match &n.props[0] {
            Prop::ArrD(v) => (v.len(), v.iter().map(|x| asc_float(*x)).collect::<Vec<_>>().join(",")),
            Prop::ArrF(v) => (v.len(), v.iter().map(|x| asc_float(*x as f64)).collect::<Vec<_>>().join(",")),
            Prop::ArrI(v) => (v.len(), v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")),
            Prop::ArrL(v) => (v.len(), v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")),
            _ => (0, String::new()),
        };
        o.push_str(&format!("{}{}: *{} {{\n{}\ta: {}\n{}}}\n", tab, n.name, len, tab, body, tab));
        return;
    }
    let props: Vec<String> = n
        .props
        .iter()
        .map(|p| match p {
            Prop::Bool(v) => (if *v { "T" } else { "F" }).to_string(),
            Prop::I32(v) => v.to_string(),
            Prop::I64(v) => v.to_string(),
            Prop::F64(v) => asc_float(*v),
            Prop::Str(s) => asc_str(s),
            Prop::Raw(_) => "\"\"".to_string(),
            _ => String::new(),
        })
        .collect();
    o.push_str(&format!("{}{}: {}", tab, n.name, props.join(", ")));
    if n.children.is_empty() {
        o.push('\n');
    } else {
        o.push_str(" {\n");
        for c in &n.children {
            asc_node(o, c, depth + 1);
        }
        o.push_str(&format!("{}}}\n", tab));
    }
}

// ====================================================================== ASCII reader

struct Asc<'a> {
    s: &'a [u8],
    p: usize,
}

impl<'a> Asc<'a> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.p).copied()
    }

    /// Skip blanks and `;` comments; newlines only when `nl`.
    fn skip(&mut self, nl: bool) {
        while let Some(c) = self.peek() {
            match c {
                b' ' | b'\t' | b'\r' => self.p += 1,
                b'\n' if nl => self.p += 1,
                b';' => {
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        self.p += 1;
                    }
                }
                _ => break,
            }
        }
    }

    fn nodes(&mut self) -> Result<Vec<FNode>, String> {
        let mut out = Vec::new();
        loop {
            self.skip(true);
            match self.peek() {
                None => break,
                Some(b'}') => {
                    self.p += 1;
                    break;
                }
                _ => {}
            }
            let st = self.p;
            while let Some(c) = self.peek() {
                if c == b':' || c == b'\n' {
                    break;
                }
                self.p += 1;
            }
            if self.peek() != Some(b':') {
                return Err(format!("Bad ASCII FBX syntax near byte {}", st));
            }
            let name = String::from_utf8_lossy(&self.s[st..self.p]).trim().to_string();
            self.p += 1;
            let mut node = FNode::new(&name);
            let mut marker = false;
            loop {
                self.skip(false);
                match self.peek() {
                    None | Some(b'\n') | Some(b'{') | Some(b'}') => break,
                    Some(b'*') => {
                        self.p += 1;
                        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                            self.p += 1;
                        }
                        marker = true;
                    }
                    Some(b'"') => {
                        self.p += 1;
                        let st = self.p;
                        while let Some(c) = self.peek() {
                            if c == b'"' {
                                break;
                            }
                            self.p += 1;
                        }
                        let t = String::from_utf8_lossy(&self.s[st..self.p]).to_string();
                        self.p += 1;
                        node.props.push(Prop::Str(t.replace("&quot;", "\"").replace("&lf;", "\n").replace("&cr;", "\r")));
                    }
                    Some(_) => {
                        let st = self.p;
                        while let Some(c) = self.peek() {
                            if matches!(c, b',' | b' ' | b'\t' | b'\r' | b'\n' | b'{' | b'}') {
                                break;
                            }
                            self.p += 1;
                        }
                        let t = String::from_utf8_lossy(&self.s[st..self.p]).to_string();
                        if let Ok(i) = t.parse::<i64>() {
                            node.props.push(Prop::I64(i));
                        } else if let Ok(f) = t.parse::<f64>() {
                            node.props.push(Prop::F64(f));
                        } else {
                            node.props.push(Prop::Str(t));
                        }
                    }
                }
                self.skip(false);
                if self.peek() == Some(b',') {
                    self.p += 1;
                    self.skip(true);
                } else {
                    break;
                }
            }
            self.skip(false);
            if self.peek() == Some(b'{') {
                self.p += 1;
                node.children = self.nodes()?;
            }
            if marker {
                let vals = node.children.iter().position(|c| c.name == "a").map(|i| node.children.remove(i)).map(|a| a.props).unwrap_or_default();
                let all_int = vals.iter().all(|v| matches!(v, Prop::I64(_)));
                node.props = if all_int {
                    vec![Prop::ArrL(vals.iter().filter_map(|v| v.i64()).collect())]
                } else {
                    vec![Prop::ArrD(vals.iter().filter_map(|v| v.f64()).collect())]
                };
            }
            out.push(node);
        }
        Ok(out)
    }
}

pub fn parse_ascii(text: &str) -> Result<Vec<FNode>, String> {
    let mut a = Asc { s: text.as_bytes(), p: 0 };
    let nodes = a.nodes()?;
    if nodes.is_empty() {
        return Err("Empty FBX file".to_string());
    }
    Ok(nodes)
}

/// Parse either flavor. Returns the tree and the declared version (0 when unknown for ASCII).
pub fn parse(data: &[u8]) -> Result<(Vec<FNode>, u32), String> {
    if is_binary(data) {
        parse_binary(data)
    } else {
        let text = String::from_utf8_lossy(data);
        let nodes = parse_ascii(&text)?;
        let v = nodes.iter().find(|n| n.name == "FBXHeaderExtension").and_then(|h| h.child("FBXVersion")).and_then(|v| v.props.first()).and_then(|p| p.i64()).unwrap_or(0) as u32;
        Ok((nodes, v))
    }
}
