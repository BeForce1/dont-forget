// PNG files for saved notes and the clipboard: 8-bit RGB out, 8-bit RGB or RGBA in (alpha is
// dropped, notes are opaque). Deflate comes from miniz_oxide; the rest is small enough to do here.
use super::draw::Image;
use miniz_oxide::{deflate::compress_to_vec_zlib, inflate::decompress_to_vec_zlib_with_limit};

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = !0u32;
    for part in parts {
        for &b in *part {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
    }
    !crc
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&[kind, data]).to_be_bytes());
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
}

/// Filters one row with `kind` (0 None, 1 Sub, 2 Up, 3 Average, 4 Paeth), bpp bytes per pixel.
fn filter(kind: u8, row: &[u8], prev: &[u8], bpp: usize, out: &mut Vec<u8>) {
    out.push(kind);
    for i in 0..row.len() {
        let a = if i >= bpp { row[i - bpp] } else { 0 };
        let (b, c) = (prev[i], if i >= bpp { prev[i - bpp] } else { 0 });
        out.push(row[i].wrapping_sub(match kind {
            1 => a,
            2 => b,
            3 => ((a as u16 + b as u16) / 2) as u8,
            4 => paeth(a, b, c),
            _ => 0,
        }));
    }
}

pub fn encode(img: &Image) -> Vec<u8> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let stride = w * 3;
    let rgb: Vec<u8> = img.pixels().iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
    let mut raw = Vec::with_capacity((stride + 1) * h);
    let zero = vec![0u8; stride];
    let mut trial = Vec::with_capacity(stride + 1);
    for y in 0..h {
        let row = &rgb[y * stride..(y + 1) * stride];
        let prev = if y == 0 { &zero[..] } else { &rgb[(y - 1) * stride..y * stride] };
        // the usual heuristic: the filter whose output has the smallest sum of signed bytes
        let mut best = (u64::MAX, 0);
        for kind in 0..5 {
            trial.clear();
            filter(kind, row, prev, 3, &mut trial);
            let cost = trial[1..].iter().map(|&b| (b as i8).unsigned_abs() as u64).sum();
            if cost < best.0 {
                best = (cost, kind);
            }
        }
        filter(best.1, row, prev, 3, &mut raw);
    }
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8 bit, RGB, deflate, adaptive filtering, no interlace
    let mut out = SIGNATURE.to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &compress_to_vec_zlib(&raw, 6));
    chunk(&mut out, b"IEND", &[]);
    out
}

pub fn decode(data: &[u8]) -> Option<Image> {
    let mut rest = data.strip_prefix(&SIGNATURE[..])?;
    let (mut w, mut h, mut bpp) = (0usize, 0usize, 0usize);
    let mut idat = Vec::new();
    while rest.len() >= 12 {
        let len = u32::from_be_bytes(rest[..4].try_into().ok()?) as usize;
        let kind = &rest[4..8];
        let body = rest.get(8..8 + len)?;
        match kind {
            b"IHDR" if len == 13 => {
                w = u32::from_be_bytes(body[..4].try_into().ok()?) as usize;
                h = u32::from_be_bytes(body[4..8].try_into().ok()?) as usize;
                // only what we write ourselves (and plain RGBA): 8 bit, no palette, no interlace
                bpp = match (body[8], body[9], body[12]) {
                    (8, 2, 0) => 3,
                    (8, 6, 0) => 4,
                    _ => return None,
                };
            }
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        rest = rest.get(12 + len..)?;
    }
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return None;
    }
    let stride = w * bpp;
    let raw = decompress_to_vec_zlib_with_limit(&idat, (stride + 1) * h).ok()?;
    if raw.len() != (stride + 1) * h {
        return None;
    }
    let mut cur = vec![0u8; stride];
    let mut prev = vec![0u8; stride];
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        let line = &raw[y * (stride + 1)..(y + 1) * (stride + 1)];
        let kind = line[0];
        for i in 0..stride {
            let a = if i >= bpp { cur[i - bpp] } else { 0 };
            let (b, c) = (prev[i], if i >= bpp { prev[i - bpp] } else { 0 });
            cur[i] = line[i + 1].wrapping_add(match kind {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((a as u16 + b as u16) / 2) as u8,
                4 => paeth(a, b, c),
                _ => return None,
            });
        }
        px.extend(cur.chunks_exact(bpp).map(|p| (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32));
        std::mem::swap(&mut cur, &mut prev);
    }
    Some(Image::from_pixels(w as u32, h as u32, px))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let (w, h) = (37, 23);
        let px: Vec<u32> = (0..w * h).map(|i| (i * 2654435761u32) & 0xFF_FFFF).collect();
        let img = Image::from_pixels(w, h, px.clone());
        let back = decode(&encode(&img)).unwrap();
        assert_eq!((back.width(), back.height()), (w, h));
        assert_eq!(back.pixels(), &px[..]);
        // flat areas compress well
        let flat = Image::new(400, 300, 0x336699);
        assert!(encode(&flat).len() < 2000);
        assert!(decode(b"not a png").is_none());
    }

    #[test]
    fn crc() {
        assert_eq!(crc32(&[b"IEND"]), 0xAE42_6082);
    }
}
