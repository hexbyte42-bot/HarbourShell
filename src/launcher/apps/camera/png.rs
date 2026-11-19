//! A tiny PNG writer (8-bit RGB, stored deflate blocks): no crate, no compression cost on
//! the CM0. Files are about the size of the raw pixels, which is fine for a few photos.

fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    for (n, slot) in t.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    t
}

fn crc32(table: &[u32; 256], parts: &[&[u8]]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for p in parts {
        for &b in *p {
            c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
        }
    }
    c ^ 0xFFFF_FFFF
}

fn chunk(out: &mut Vec<u8>, table: &[u32; 256], kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(kind);
    out.extend(data);
    out.extend(crc32(table, &[kind, data]).to_be_bytes());
}

/// Encode tightly packed RGB as a PNG file.
pub fn encode_rgb(w: u32, h: u32, rgb: &[u8]) -> Vec<u8> {
    assert_eq!(rgb.len(), (w * h * 3) as usize);
    let table = crc_table();
    // zlib stream: header, stored blocks of up to 65535 bytes, Adler-32.
    let row = w as usize * 3;
    let mut raw = Vec::with_capacity((row + 1) * h as usize);
    for y in 0..h as usize {
        raw.push(0); // filter: none
        raw.extend_from_slice(&rgb[y * row..(y + 1) * row]);
    }
    let mut z = vec![0x78, 0x01];
    let blocks = raw.chunks(65535).count().max(1);
    for (i, b) in raw.chunks(65535).enumerate() {
        z.push((i + 1 == blocks) as u8);
        z.extend((b.len() as u16).to_le_bytes());
        z.extend((!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    let (mut s1, mut s2) = (1u32, 0u32);
    for &b in &raw {
        s1 = (s1 + b as u32) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    z.extend(((s2 << 16) | s1).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    chunk(&mut out, &table, b"IHDR", &ihdr);
    chunk(&mut out, &table, b"IDAT", &z);
    chunk(&mut out, &table, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse the PNG back (chunks, CRCs, stored blocks) and return (w, h, rgb).
    pub fn parse(png: &[u8]) -> (u32, u32, Vec<u8>) {
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 13, 10, 26, 10]);
        let table = crc_table();
        let (mut i, mut w, mut h, mut idat) = (8, 0, 0, Vec::new());
        while i < png.len() {
            let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
            let kind: [u8; 4] = png[i + 4..i + 8].try_into().unwrap();
            let data = &png[i + 8..i + 8 + len];
            let crc = u32::from_be_bytes(png[i + 8 + len..i + 12 + len].try_into().unwrap());
            assert_eq!(crc, crc32(&table, &[&kind, data]), "crc of {}", String::from_utf8_lossy(&kind));
            match &kind {
                b"IHDR" => {
                    w = u32::from_be_bytes(data[0..4].try_into().unwrap());
                    h = u32::from_be_bytes(data[4..8].try_into().unwrap());
                    assert_eq!(&data[8..], &[8, 2, 0, 0, 0]);
                }
                b"IDAT" => idat.extend_from_slice(data),
                _ => {}
            }
            i += 12 + len;
        }
        let mut raw = Vec::new();
        let mut p = 2;
        loop {
            let last = idat[p] & 1;
            let n = u16::from_le_bytes([idat[p + 1], idat[p + 2]]) as usize;
            assert_eq!(!u16::from_le_bytes([idat[p + 1], idat[p + 2]]), u16::from_le_bytes([idat[p + 3], idat[p + 4]]));
            raw.extend_from_slice(&idat[p + 5..p + 5 + n]);
            p += 5 + n;
            if last == 1 {
                break;
            }
        }
        let row = w as usize * 3;
        let mut rgb = Vec::new();
        for y in 0..h as usize {
            assert_eq!(raw[y * (row + 1)], 0);
            rgb.extend_from_slice(&raw[y * (row + 1) + 1..(y + 1) * (row + 1)]);
        }
        (w, h, rgb)
    }

    #[test]
    fn round_trips_a_small_image() {
        let rgb: Vec<u8> = (0..4 * 3 * 3).map(|i| (i * 7) as u8).collect();
        let (w, h, back) = parse(&encode_rgb(4, 3, &rgb));
        assert_eq!((w, h), (4, 3));
        assert_eq!(back, rgb);
    }

    #[test]
    fn large_images_use_several_stored_blocks() {
        let rgb: Vec<u8> = (0..200 * 150 * 3).map(|i| (i % 251) as u8).collect();
        let png = encode_rgb(200, 150, &rgb);
        assert!(png.len() > rgb.len());
        assert_eq!(parse(&png).2, rgb);
    }

    #[test]
    fn crc32_matches_the_known_check_value() {
        assert_eq!(crc32(&crc_table(), &[b"123456789"]), 0xCBF4_3926);
    }
}
