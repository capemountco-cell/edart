//! A small baseline JPEG encoder (8-bit, YCbCr 4:2:0, the standard Huffman
//! tables).
//!
//! The mirror needs one thing from an image library: turn a frame into a
//! JPEG, about ten times a second for the preview and once per agent step.
//! That is this file. Written here instead of calling ImageIO so that it is
//! tested on Linux, where the tests decode its output.

use crate::image::Rgb;

/// Zigzag position → index in the 8x8 block (row-major).
#[rustfmt::skip]
const ZIGZAG: [usize; 64] = [
     0,  1,  8, 16,  9,  2,  3, 10,
    17, 24, 32, 25, 18, 11,  4,  5,
    12, 19, 26, 33, 40, 48, 41, 34,
    27, 20, 13,  6,  7, 14, 21, 28,
    35, 42, 49, 56, 57, 50, 43, 36,
    29, 22, 15, 23, 30, 37, 44, 51,
    58, 59, 52, 45, 38, 31, 39, 46,
    53, 60, 61, 54, 47, 55, 62, 63,
];

/// The JPEG standard's example quantisation tables (Annex K), row-major.
#[rustfmt::skip]
const LUMA_QUANT: [u16; 64] = [
    16, 11, 10, 16,  24,  40,  51,  61,
    12, 12, 14, 19,  26,  58,  60,  55,
    14, 13, 16, 24,  40,  57,  69,  56,
    14, 17, 22, 29,  51,  87,  80,  62,
    18, 22, 37, 56,  68, 109, 103,  77,
    24, 35, 55, 64,  81, 104, 113,  92,
    49, 64, 78, 87, 103, 121, 120, 101,
    72, 92, 95, 98, 112, 100, 103,  99,
];
#[rustfmt::skip]
const CHROMA_QUANT: [u16; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99,
    18, 21, 26, 66, 99, 99, 99, 99,
    24, 26, 56, 99, 99, 99, 99, 99,
    47, 66, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99,
];

/// A Huffman table as JPEG stores it: how many codes of each length
/// (1..=16), then the symbols in code order.
struct HuffSpec {
    counts: [u8; 16],
    symbols: &'static [u8],
}

const DC_SYMBOLS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

const DC_LUMA: HuffSpec = HuffSpec {
    counts: [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0],
    symbols: &DC_SYMBOLS,
};
const DC_CHROMA: HuffSpec = HuffSpec {
    counts: [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0],
    symbols: &DC_SYMBOLS,
};
#[rustfmt::skip]
const AC_LUMA: HuffSpec = HuffSpec {
    counts: [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d],
    symbols: &[
        0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61,
        0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52,
        0xd1, 0xf0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25,
        0x26, 0x27, 0x28, 0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45,
        0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64,
        0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83,
        0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99,
        0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6,
        0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3,
        0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8,
        0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
    ],
};
#[rustfmt::skip]
const AC_CHROMA: HuffSpec = HuffSpec {
    counts: [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77],
    symbols: &[
        0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61,
        0x71, 0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33,
        0x52, 0xf0, 0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18,
        0x19, 0x1a, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44,
        0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63,
        0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a,
        0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97,
        0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4,
        0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca,
        0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7,
        0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
    ],
};

/// Code and length for each symbol, derived from a [`HuffSpec`] the way
/// every decoder derives them (canonical codes, shortest first).
struct HuffCodes {
    code: [u16; 256],
    len: [u8; 256],
}

impl HuffCodes {
    fn new(spec: &HuffSpec) -> Self {
        let mut codes = HuffCodes {
            code: [0; 256],
            len: [0; 256],
        };
        let mut code = 0u16;
        let mut next = 0usize;
        for (i, &count) in spec.counts.iter().enumerate() {
            for _ in 0..count {
                let symbol = usize::from(spec.symbols[next]);
                codes.code[symbol] = code;
                codes.len[symbol] = i as u8 + 1;
                code += 1;
                next += 1;
            }
            code <<= 1;
        }
        codes
    }
}

/// Writes bits most significant first, stuffing a zero byte after 0xFF as
/// the format requires.
struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    bits: u32,
}

impl BitWriter {
    fn put(&mut self, code: u32, len: u32) {
        if len == 0 {
            return;
        }
        self.acc = (self.acc << len) | (code & ((1 << len) - 1));
        self.bits += len;
        while self.bits >= 8 {
            let byte = (self.acc >> (self.bits - 8)) as u8;
            self.out.push(byte);
            if byte == 0xff {
                self.out.push(0);
            }
            self.bits -= 8;
        }
    }

    fn flush(&mut self) {
        if self.bits > 0 {
            let pad = 8 - self.bits;
            self.put((1 << pad) - 1, pad);
        }
    }
}

/// Scale a base table for a quality of 1..=100 (libjpeg's formula).
fn quant_table(base: &[u16; 64], quality: u8) -> [u16; 64] {
    let quality = u32::from(quality.clamp(1, 100));
    let scale = if quality < 50 {
        5000 / quality
    } else {
        200 - quality * 2
    };
    let mut table = [0u16; 64];
    for (out, &value) in table.iter_mut().zip(base) {
        *out = ((u32::from(value) * scale + 50) / 100).clamp(1, 255) as u16;
    }
    table
}

/// `basis[u][x]` of the 8-point DCT, including the normalisation.
fn dct_basis() -> [[f32; 8]; 8] {
    let mut basis = [[0f32; 8]; 8];
    for (u, row) in basis.iter_mut().enumerate() {
        let scale = if u == 0 { (1.0f32 / 8.0).sqrt() } else { 0.5 };
        for (x, value) in row.iter_mut().enumerate() {
            let angle = (2 * x + 1) as f32 * u as f32 * std::f32::consts::PI / 16.0;
            *value = scale * angle.cos();
        }
    }
    basis
}

/// Forward DCT of one block, in place: rows, then columns.
fn dct(block: &mut [f32; 64], basis: &[[f32; 8]; 8]) {
    let mut tmp = [0f32; 64];
    for y in 0..8 {
        let row = &block[y * 8..y * 8 + 8];
        for u in 0..8 {
            let mut sum = 0.0;
            for x in 0..8 {
                sum += row[x] * basis[u][x];
            }
            tmp[y * 8 + u] = sum;
        }
    }
    for u in 0..8 {
        for v in 0..8 {
            let mut sum = 0.0;
            for y in 0..8 {
                sum += tmp[y * 8 + u] * basis[v][y];
            }
            block[v * 8 + u] = sum;
        }
    }
}

/// Bit length of a coefficient's magnitude, and the bits JPEG stores for it.
fn magnitude(value: i32) -> (u32, u32) {
    let abs = value.unsigned_abs();
    let size = 32 - abs.leading_zeros();
    let bits = if value < 0 {
        (value - 1) as u32 & ((1 << size) - 1)
    } else {
        value as u32
    };
    (size, bits)
}

struct Component<'a> {
    quant: &'a [u16; 64],
    dc: &'a HuffCodes,
    ac: &'a HuffCodes,
}

/// Transform, quantise and entropy-code one block. Returns its DC value,
/// which the next block of the same component is coded against.
fn encode_block(
    writer: &mut BitWriter,
    block: &mut [f32; 64],
    basis: &[[f32; 8]; 8],
    component: &Component,
    previous_dc: i32,
) -> i32 {
    dct(block, basis);
    let mut quantised = [0i32; 64];
    for (i, &at) in ZIGZAG.iter().enumerate() {
        quantised[i] = (block[at] / f32::from(component.quant[at])).round() as i32;
    }

    let (size, bits) = magnitude(quantised[0] - previous_dc);
    writer.put(
        u32::from(component.dc.code[size as usize]),
        u32::from(component.dc.len[size as usize]),
    );
    writer.put(bits, size);

    let last = quantised.iter().rposition(|&v| v != 0).unwrap_or(0);
    let mut run = 0u32;
    for &value in &quantised[1..=last] {
        if value == 0 {
            run += 1;
            continue;
        }
        while run >= 16 {
            // ZRL: sixteen zeros.
            writer.put(
                u32::from(component.ac.code[0xf0]),
                u32::from(component.ac.len[0xf0]),
            );
            run -= 16;
        }
        let (size, bits) = magnitude(value);
        let symbol = (run << 4 | size) as usize;
        writer.put(
            u32::from(component.ac.code[symbol]),
            u32::from(component.ac.len[symbol]),
        );
        writer.put(bits, size);
        run = 0;
    }
    if last < 63 {
        // EOB: the rest is zero.
        writer.put(
            u32::from(component.ac.code[0]),
            u32::from(component.ac.len[0]),
        );
    }
    quantised[0]
}

fn segment(out: &mut Vec<u8>, marker: u8, payload: &[u8]) {
    out.extend_from_slice(&[0xff, marker]);
    out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(payload);
}

fn huffman_segment(out: &mut Vec<u8>, class_and_id: u8, spec: &HuffSpec) {
    let mut payload = vec![class_and_id];
    payload.extend_from_slice(&spec.counts);
    payload.extend_from_slice(spec.symbols);
    segment(out, 0xc4, &payload);
}

/// Encode `image` as a JPEG. `quality` is 1..=100; 80 is a good default.
///
/// Returns `None` for an empty image, one larger than JPEG's 65535 pixels a
/// side, or one whose data does not match its size.
pub fn encode(image: &Rgb, quality: u8) -> Option<Vec<u8>> {
    let (width, height) = (image.width, image.height);
    if width == 0
        || height == 0
        || width > 65535
        || height > 65535
        || image.data.len() != width * height * 3
    {
        return None;
    }
    let luma_quant = quant_table(&LUMA_QUANT, quality);
    let chroma_quant = quant_table(&CHROMA_QUANT, quality);

    let mut out = Vec::with_capacity(width * height / 4 + 1024);
    out.extend_from_slice(&[0xff, 0xd8]);
    // JFIF header: version 1.1, no density, no thumbnail.
    segment(
        &mut out,
        0xe0,
        &[b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0],
    );
    for (id, table) in [(0u8, &luma_quant), (1, &chroma_quant)] {
        let mut payload = vec![id];
        payload.extend(ZIGZAG.iter().map(|&at| table[at] as u8));
        segment(&mut out, 0xdb, &payload);
    }
    // Baseline frame: Y at full size (2x2 samples per MCU), Cb and Cr halved.
    let mut frame = vec![8];
    frame.extend_from_slice(&(height as u16).to_be_bytes());
    frame.extend_from_slice(&(width as u16).to_be_bytes());
    frame.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    segment(&mut out, 0xc0, &frame);
    huffman_segment(&mut out, 0x00, &DC_LUMA);
    huffman_segment(&mut out, 0x10, &AC_LUMA);
    huffman_segment(&mut out, 0x01, &DC_CHROMA);
    huffman_segment(&mut out, 0x11, &AC_CHROMA);
    segment(&mut out, 0xda, &[3, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0]);

    let dc_luma = HuffCodes::new(&DC_LUMA);
    let ac_luma = HuffCodes::new(&AC_LUMA);
    let dc_chroma = HuffCodes::new(&DC_CHROMA);
    let ac_chroma = HuffCodes::new(&AC_CHROMA);
    let luma = Component {
        quant: &luma_quant,
        dc: &dc_luma,
        ac: &ac_luma,
    };
    let chroma = Component {
        quant: &chroma_quant,
        dc: &dc_chroma,
        ac: &ac_chroma,
    };
    let basis = dct_basis();
    let mut writer = BitWriter {
        out,
        acc: 0,
        bits: 0,
    };
    let (mut dc_y, mut dc_cb, mut dc_cr) = (0, 0, 0);
    let mut y = [0f32; 256];
    let mut cb = [0f32; 256];
    let mut cr = [0f32; 256];
    let mut block = [0f32; 64];

    for top in (0..height).step_by(16) {
        for left in (0..width).step_by(16) {
            // The 16x16 pixels of this MCU, repeating the edge past it.
            for dy in 0..16 {
                let row = (top + dy).min(height - 1) * width;
                for dx in 0..16 {
                    let at = (row + (left + dx).min(width - 1)) * 3;
                    let r = f32::from(image.data[at]);
                    let g = f32::from(image.data[at + 1]);
                    let b = f32::from(image.data[at + 2]);
                    let i = dy * 16 + dx;
                    y[i] = 0.299 * r + 0.587 * g + 0.114 * b - 128.0;
                    cb[i] = -0.168_736 * r - 0.331_264 * g + 0.5 * b;
                    cr[i] = 0.5 * r - 0.418_688 * g - 0.081_312 * b;
                }
            }
            for (by, bx) in [(0, 0), (0, 8), (8, 0), (8, 8)] {
                for row in 0..8 {
                    let from = (by + row) * 16 + bx;
                    block[row * 8..row * 8 + 8].copy_from_slice(&y[from..from + 8]);
                }
                dc_y = encode_block(&mut writer, &mut block, &basis, &luma, dc_y);
            }
            for (plane, dc) in [(&cb, &mut dc_cb), (&cr, &mut dc_cr)] {
                for row in 0..8 {
                    for col in 0..8 {
                        let at = row * 32 + col * 2;
                        block[row * 8 + col] =
                            (plane[at] + plane[at + 1] + plane[at + 16] + plane[at + 17]) / 4.0;
                    }
                }
                *dc = encode_block(&mut writer, &mut block, &basis, &chroma, *dc);
            }
        }
    }
    writer.flush();
    let mut out = writer.out;
    out.extend_from_slice(&[0xff, 0xd9]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: usize, height: usize) -> Rgb {
        let mut data = Vec::with_capacity(width * height * 3);
        for y in 0..height {
            for x in 0..width {
                data.extend_from_slice(&[
                    (x * 255 / width.max(1)) as u8,
                    (y * 255 / height.max(1)) as u8,
                    ((x + y) % 256) as u8,
                ]);
            }
        }
        Rgb {
            width,
            height,
            data,
        }
    }

    /// Walk the segments up to the scan and return (marker, payload) pairs.
    fn segments(jpeg: &[u8]) -> Vec<(u8, &[u8])> {
        assert_eq!(&jpeg[..2], [0xff, 0xd8]);
        let mut at = 2;
        let mut found = Vec::new();
        loop {
            assert_eq!(jpeg[at], 0xff);
            let marker = jpeg[at + 1];
            let len = usize::from(u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]));
            found.push((marker, &jpeg[at + 4..at + 2 + len]));
            at += 2 + len;
            if marker == 0xda {
                return found;
            }
        }
    }

    #[test]
    fn the_file_has_the_baseline_structure_and_the_right_size() {
        let jpeg = encode(&gradient(390, 844), 80).unwrap();
        let found = segments(&jpeg);
        let markers: Vec<u8> = found.iter().map(|(m, _)| *m).collect();
        assert_eq!(
            markers,
            [0xe0, 0xdb, 0xdb, 0xc0, 0xc4, 0xc4, 0xc4, 0xc4, 0xda]
        );
        let frame = found[3].1;
        assert_eq!(frame[0], 8);
        assert_eq!(u16::from_be_bytes([frame[1], frame[2]]), 844);
        assert_eq!(u16::from_be_bytes([frame[3], frame[4]]), 390);
        assert_eq!(&jpeg[jpeg.len() - 2..], [0xff, 0xd9]);
    }

    #[test]
    fn huffman_tables_are_complete_and_consistent() {
        for (spec, symbols) in [
            (&DC_LUMA, 12),
            (&DC_CHROMA, 12),
            (&AC_LUMA, 162),
            (&AC_CHROMA, 162),
        ] {
            let total: usize = spec.counts.iter().map(|&c| usize::from(c)).sum();
            assert_eq!(total, symbols);
            assert_eq!(spec.symbols.len(), symbols);
            let mut seen = [false; 256];
            for &s in spec.symbols {
                assert!(!seen[usize::from(s)], "duplicate symbol {s:#x}");
                seen[usize::from(s)] = true;
            }
            // Kraft sum below 1: the codes are a valid prefix code.
            let kraft: u32 = spec
                .counts
                .iter()
                .enumerate()
                .map(|(i, &c)| u32::from(c) << (15 - i))
                .sum();
            assert!(kraft < 1 << 16);
        }
        // Every run/size pair the encoder can emit has a code.
        let ac = HuffCodes::new(&AC_LUMA);
        for run in 0..16usize {
            for size in 1..=10usize {
                assert!(ac.len[run << 4 | size] > 0);
            }
        }
        assert!(ac.len[0x00] > 0 && ac.len[0xf0] > 0);
    }

    #[test]
    fn no_marker_appears_inside_the_scan() {
        // Noise makes many 0xFF bytes; each must be followed by a stuffed 0.
        let mut seed = 0x2545_f491u32;
        let data: Vec<u8> = (0..64 * 48 * 3)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed as u8
            })
            .collect();
        let image = Rgb {
            width: 64,
            height: 48,
            data,
        };
        let jpeg = encode(&image, 95).unwrap();
        let header: usize = 2 + segments(&jpeg)
            .iter()
            .map(|(_, payload)| payload.len() + 4)
            .sum::<usize>();
        let scan = &jpeg[header..jpeg.len() - 2];
        assert!(scan.len() > 1000);
        for pair in scan.windows(2) {
            if pair[0] == 0xff {
                assert_eq!(pair[1], 0);
            }
        }
        assert_ne!(scan.last(), Some(&0xff));
    }

    #[test]
    fn sizes_that_are_not_multiples_of_sixteen_encode() {
        for (w, h) in [(1, 1), (7, 3), (17, 33), (415, 900)] {
            let jpeg = encode(&gradient(w, h), 75).unwrap();
            assert_eq!(&jpeg[jpeg.len() - 2..], [0xff, 0xd9]);
        }
    }

    #[test]
    fn a_flat_image_is_tiny_and_quality_changes_the_size() {
        let flat = Rgb {
            width: 320,
            height: 320,
            data: vec![128; 320 * 320 * 3],
        };
        assert!(encode(&flat, 80).unwrap().len() < 4000);
        let low = encode(&gradient(320, 320), 30).unwrap().len();
        let high = encode(&gradient(320, 320), 95).unwrap().len();
        assert!(low < high);
    }

    #[test]
    fn bad_images_are_refused() {
        let empty = Rgb {
            width: 0,
            height: 0,
            data: vec![],
        };
        assert!(encode(&empty, 80).is_none());
        let short = Rgb {
            width: 4,
            height: 4,
            data: vec![0; 10],
        };
        assert!(encode(&short, 80).is_none());
    }

    #[test]
    fn magnitudes_follow_the_standard() {
        assert_eq!(magnitude(0), (0, 0));
        assert_eq!(magnitude(1), (1, 1));
        assert_eq!(magnitude(-1), (1, 0));
        assert_eq!(magnitude(5), (3, 5));
        assert_eq!(magnitude(-5), (3, 2));
        assert_eq!(magnitude(-255), (8, 0));
    }

    /// Writes a sample for an outside decoder when EDART_JPEG_SAMPLE names a
    /// file; a no-op otherwise.
    #[test]
    fn sample_for_an_outside_decoder() {
        if let Ok(path) = std::env::var("EDART_JPEG_SAMPLE") {
            std::fs::write(path, encode(&gradient(415, 900), 80).unwrap()).unwrap();
        }
    }
}
