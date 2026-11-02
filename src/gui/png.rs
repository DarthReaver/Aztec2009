//! Minimal PNG writer (RGBA8, uncompressed deflate blocks) so renders can be saved without extra crates.

fn crc32(data: &[u8], mut crc: u32) -> u32 {
    crc = !crc;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &d in data {
        a = (a + d as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    let c = crc32(body, crc32(kind, 0));
    out.extend_from_slice(&c.to_be_bytes());
}

/// Encode `rgba` (w*h*4 bytes) as a PNG file.
pub fn encode_rgba(w: usize, h: usize, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w * 4 + 1) * h);
    for y in 0..h {
        raw.push(0);
        raw.extend_from_slice(&rgba[y * w * 4..(y + 1) * w * 4]);
    }
    let mut z = vec![0x78, 0x01];
    let mut i = 0;
    while i < raw.len() {
        let n = (raw.len() - i).min(65535);
        let last = i + n >= raw.len();
        z.push(if last { 1 } else { 0 });
        z.extend_from_slice(&(n as u16).to_le_bytes());
        z.extend_from_slice(&(!(n as u16)).to_le_bytes());
        z.extend_from_slice(&raw[i..i + n]);
        i += n;
    }
    if raw.is_empty() {
        z.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Procedural application icon: a dark rounded tile with an orange wire cube.
pub fn app_icon(size: usize) -> Vec<u8> {
    let mut px = vec![0u8; size * size * 4];
    let s = size as f32;
    let set = |px: &mut Vec<u8>, x: i32, y: i32, c: [u8; 3], a: f32| {
        if x < 0 || y < 0 || x >= size as i32 || y >= size as i32 {
            return;
        }
        let i = (y as usize * size + x as usize) * 4;
        let old_a = px[i + 3] as f32 / 255.0;
        let na = a + old_a * (1.0 - a);
        for k in 0..3 {
            let o = px[i + k] as f32;
            px[i + k] = ((c[k] as f32 * a + o * old_a * (1.0 - a)) / na.max(1e-4)).clamp(0.0, 255.0) as u8;
        }
        px[i + 3] = (na * 255.0) as u8;
    };
    // rounded tile with vertical gradient
    let r = s * 0.18;
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let cx = fx.clamp(r, s - r);
            let cy = fy.clamp(r, s - r);
            let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            let cov = (r - d + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let t = fy / s;
                let c = [(70.0 - 30.0 * t) as u8, (74.0 - 30.0 * t) as u8, (82.0 - 30.0 * t) as u8];
                set(&mut px, x as i32, y as i32, c, cov);
            }
        }
    }
    // isometric cube wireframe
    let c = (s * 0.5, s * 0.52);
    let k = s * 0.26;
    let pts = |x: f32, y: f32, z: f32| {
        // x right-down, z left-down, y up (iso)
        (c.0 + (x - z) * k * 0.8, c.1 + (x + z) * k * 0.46 - y * k * 0.95)
    };
    let v = |i: u8| pts((i & 1) as f32 - 0.5, ((i >> 1) & 1) as f32 - 0.5, ((i >> 2) & 1) as f32 - 0.5);
    let edges: [(u8, u8); 12] = [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];
    for (i, j) in edges {
        let (a, b) = (v(i), v(j));
        let n = 200;
        for t in 0..=n {
            let u = t as f32 / n as f32;
            let (x, y) = (a.0 + (b.0 - a.0) * u, a.1 + (b.1 - a.1) * u);
            let w = s * 0.018;
            for oy in -2..=2 {
                for ox in -2..=2 {
                    let (px_, py_) = (x.floor() as i32 + ox, y.floor() as i32 + oy);
                    let dd = (((px_ as f32 + 0.5) - x).powi(2) + ((py_ as f32 + 0.5) - y).powi(2)).sqrt();
                    let cov = (w + 0.5 - dd).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        set(&mut px, px_, py_, [255, 150, 40], cov * 0.6);
                    }
                }
            }
        }
    }
    px
}
