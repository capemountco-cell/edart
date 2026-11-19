//! A frame of the iPhone's screen as plain RGB bytes, and the two things
//! done to it before it leaves the capture thread: shrinking and checking
//! whether the screen is off.
//!
//! Kept free of any macOS type so it is tested on Linux. The capture code
//! hands over the locked pixel buffer's bytes and gets one of these back.

/// An image, 3 bytes per pixel (R, G, B), rows top to bottom, no padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgb {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

/// How the source bytes are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// 4 bytes per pixel: blue, green, red, unused (what AVFoundation
    /// delivers for `kCVPixelFormatType_32BGRA`).
    Bgra,
    /// 3 bytes per pixel: red, green, blue.
    Rgb,
}

impl Layout {
    fn bytes_per_pixel(self) -> usize {
        match self {
            Layout::Bgra => 4,
            Layout::Rgb => 3,
        }
    }

    /// Offsets of red, green and blue inside a pixel.
    fn offsets(self) -> (usize, usize, usize) {
        match self {
            Layout::Bgra => (2, 1, 0),
            Layout::Rgb => (0, 1, 2),
        }
    }
}

/// The size an image has after shrinking so its longer edge is at most
/// `max_edge`. Never enlarges; never returns a zero side.
pub fn fit(width: usize, height: usize, max_edge: usize) -> (usize, usize) {
    let longer = width.max(height);
    if longer <= max_edge || longer == 0 || max_edge == 0 {
        return (width, height);
    }
    let scale = |side: usize| ((side * max_edge + longer / 2) / longer).max(1);
    (scale(width), scale(height))
}

/// Shrink `src` so its longer edge is at most `max_edge`, averaging the
/// source pixels under each output pixel. `stride` is the bytes per source
/// row (at least `width` pixels' worth; rows may be padded).
///
/// Returns `None` when the buffer is too short for the stated size, so a
/// bad frame is dropped instead of read out of bounds.
pub fn shrink(
    src: &[u8],
    width: usize,
    height: usize,
    stride: usize,
    layout: Layout,
    max_edge: usize,
) -> Option<Rgb> {
    let bpp = layout.bytes_per_pixel();
    if width == 0 || height == 0 || stride < width.checked_mul(bpp)? {
        return None;
    }
    let needed = stride.checked_mul(height - 1)?.checked_add(width * bpp)?;
    if src.len() < needed {
        return None;
    }
    let (r, g, b) = layout.offsets();
    let (out_w, out_h) = fit(width, height, max_edge);
    let mut data = Vec::with_capacity(out_w * out_h * 3);

    if (out_w, out_h) == (width, height) {
        for y in 0..height {
            let row = &src[y * stride..y * stride + width * bpp];
            for pixel in row.chunks_exact(bpp) {
                data.extend_from_slice(&[pixel[r], pixel[g], pixel[b]]);
            }
        }
        return Some(Rgb {
            width,
            height,
            data,
        });
    }

    // The source columns under each output column: [start, end).
    let spans: Vec<(usize, usize)> = (0..out_w)
        .map(|x| {
            let start = x * width / out_w;
            let end = ((x + 1) * width / out_w).max(start + 1).min(width);
            (start, end)
        })
        .collect();
    let mut sums = vec![[0u32; 3]; out_w];
    for y in 0..out_h {
        let top = y * height / out_h;
        let bottom = ((y + 1) * height / out_h).max(top + 1).min(height);
        sums.iter_mut().for_each(|s| *s = [0; 3]);
        for sy in top..bottom {
            let row = &src[sy * stride..sy * stride + width * bpp];
            for (sum, &(start, end)) in sums.iter_mut().zip(&spans) {
                for pixel in row[start * bpp..end * bpp].chunks_exact(bpp) {
                    sum[0] += u32::from(pixel[r]);
                    sum[1] += u32::from(pixel[g]);
                    sum[2] += u32::from(pixel[b]);
                }
            }
        }
        let rows = (bottom - top) as u32;
        for (sum, &(start, end)) in sums.iter().zip(&spans) {
            let count = rows * (end - start) as u32;
            let half = count / 2;
            data.extend_from_slice(&[
                ((sum[0] + half) / count) as u8,
                ((sum[1] + half) / count) as u8,
                ((sum[2] + half) / count) as u8,
            ]);
        }
    }
    Some(Rgb {
        width: out_w,
        height: out_h,
        data,
    })
}

impl Rgb {
    /// A copy whose longer edge is at most `max_edge`.
    pub fn shrunk(&self, max_edge: usize) -> Rgb {
        shrink(
            &self.data,
            self.width,
            self.height,
            self.width * 3,
            Layout::Rgb,
            max_edge,
        )
        .unwrap_or_else(|| self.clone())
    }

    /// True when the picture is black all over: the iPhone's screen is off
    /// or it is sending nothing yet. The capture succeeds either way, so
    /// this is the only way to tell (idea from iPhone Use's `isDark`).
    pub fn is_dark(&self) -> bool {
        if self.width == 0 || self.height == 0 {
            return true;
        }
        // About 48 x 104 samples, the grid iPhone Use compares screens on.
        let step_x = (self.width / 48).max(1);
        let step_y = (self.height / 104).max(1);
        let mut total = 0usize;
        let mut lit = 0usize;
        for y in (0..self.height).step_by(step_y) {
            for x in (0..self.width).step_by(step_x) {
                let i = (y * self.width + x) * 3;
                let luma = (u32::from(self.data[i]) * 299
                    + u32::from(self.data[i + 1]) * 587
                    + u32::from(self.data[i + 2]) * 114)
                    / 1000;
                total += 1;
                if luma >= 12 {
                    lit += 1;
                }
            }
        }
        // A stray lit sample (a status dot) does not make a screen "on".
        lit * 200 <= total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(width: usize, height: usize, rgb: [u8; 3]) -> Rgb {
        Rgb {
            width,
            height,
            data: rgb.repeat(width * height),
        }
    }

    #[test]
    fn fit_keeps_small_images_and_the_aspect_ratio() {
        assert_eq!(fit(390, 844, 1600), (390, 844));
        assert_eq!(fit(1170, 2532, 1600), (739, 1600));
        assert_eq!(fit(2532, 1170, 1600), (1600, 739));
        assert_eq!(fit(4000, 1, 100), (100, 1));
        assert_eq!(fit(0, 0, 100), (0, 0));
    }

    #[test]
    fn bgra_is_reordered_and_padding_is_skipped() {
        // 2x2, stride 12 (one padding pixel per row).
        #[rustfmt::skip]
        let src = [
            1, 2, 3, 255,   4, 5, 6, 255,   99, 99, 99, 99,
            7, 8, 9, 255,   10, 11, 12, 255, 99, 99, 99, 99,
        ];
        let image = shrink(&src, 2, 2, 12, Layout::Bgra, 100).unwrap();
        assert_eq!(image.data, [3, 2, 1, 6, 5, 4, 9, 8, 7, 12, 11, 10]);
    }

    #[test]
    fn shrinking_averages_the_pixels_under_each_output_pixel() {
        // 4x2 → 2x1: each output pixel is the mean of a 2x2 block.
        #[rustfmt::skip]
        let src = [
            0, 0, 0,   100, 0, 0,   10, 10, 10,   30, 30, 30,
            0, 0, 0,   100, 0, 0,   10, 10, 10,   30, 30, 30,
        ];
        let image = shrink(&src, 4, 2, 12, Layout::Rgb, 2).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.data, [50, 0, 0, 20, 20, 20]);
    }

    #[test]
    fn odd_sizes_shrink_without_reading_out_of_bounds() {
        let image = flat(1179, 2556, [200, 100, 50]);
        let small = image.shrunk(900);
        assert_eq!((small.width, small.height), (415, 900));
        assert_eq!(small.data.len(), 415 * 900 * 3);
        assert!(small.data.chunks_exact(3).all(|p| p == [200, 100, 50]));
    }

    #[test]
    fn a_short_buffer_is_refused() {
        assert!(shrink(&[0; 10], 2, 2, 8, Layout::Bgra, 100).is_none());
        assert!(shrink(&[0; 16], 2, 2, 4, Layout::Bgra, 100).is_none());
        assert!(shrink(&[], 0, 0, 0, Layout::Bgra, 100).is_none());
        // The last row needs no padding.
        assert!(shrink(&[0; 20], 2, 2, 12, Layout::Bgra, 100).is_some());
    }

    #[test]
    fn a_black_frame_is_dark_and_a_lit_one_is_not() {
        assert!(flat(390, 844, [0, 0, 0]).is_dark());
        assert!(flat(390, 844, [8, 8, 8]).is_dark());
        assert!(!flat(390, 844, [40, 40, 40]).is_dark());
        // One lit corner pixel is still an off screen.
        let mut nearly = flat(390, 844, [0, 0, 0]);
        nearly.data[0..3].copy_from_slice(&[255, 255, 255]);
        assert!(nearly.is_dark());
        // A lit top half is on.
        let mut half = flat(100, 100, [0, 0, 0]);
        half.data[..100 * 50 * 3].fill(200);
        assert!(!half.is_dark());
    }
}
