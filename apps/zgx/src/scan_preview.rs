//! The camera's live picture while a connect code is scanned
//! (src/android_camera.rs): a frame's brightness made smaller and turned
//! upright, as the RGBA picture the scan view draws
//! (ui/screens/settings/phone_join.luau, `phone.scan_preview`). Only the
//! newest is held, and only while a scan runs; nothing of it is kept or
//! logged.

/// The preview's longest side at most, in pixels.
pub const SIDE: usize = 320;

/// How far to turn a frame clockwise, in degrees, for it to stand upright
/// on the screen: the camera's `sensor` orientation (ACAMERA_SENSOR_ORIENTATION)
/// less the display's rotation (`display`, degrees), or plus it for a
/// front camera (whose frame is mirrored too).
pub fn turn(sensor: i32, display: i32, front: bool) -> i32 {
    let turn = if front { sensor + display } else { sensor - display };
    turn.rem_euclid(360) / 90 * 90
}

/// `grey` (`w` by `h`, row after row) shrunk by a whole factor until its
/// longest side is at most [SIDE] (each pixel the mean of its block),
/// mirrored if `mirror`, then turned clockwise by `degrees` (a multiple of
/// 90): its width, height and opaque RGBA pixels.
pub fn preview(w: usize, h: usize, grey: &[u8], degrees: i32, mirror: bool) -> Option<(u32, u32, Vec<u8>)> {
    if w == 0 || h == 0 || grey.len() < w * h {
        return None;
    }
    let step = w.max(h).div_ceil(SIDE).max(1);
    let (sw, sh) = (w / step, h / step);
    if sw == 0 || sh == 0 {
        return None;
    }
    // The shrunk frame, mirrored as asked.
    let mut small = vec![0u8; sw * sh];
    let area = (step * step) as u32;
    for y in 0..sh {
        for x in 0..sw {
            let mut sum = 0u32;
            for dy in 0..step {
                let row = (y * step + dy) * w + x * step;
                sum += grey[row..row + step].iter().map(|&v| v as u32).sum::<u32>();
            }
            let at = if mirror { sw - 1 - x } else { x };
            small[y * sw + at] = (sum / area) as u8;
        }
    }
    let degrees = degrees.rem_euclid(360) / 90 * 90;
    let (ow, oh) = if degrees % 180 == 0 { (sw, sh) } else { (sh, sw) };
    let mut rgba = Vec::with_capacity(ow * oh * 4);
    for y in 0..oh {
        for x in 0..ow {
            // The source pixel that lands at (x, y).
            let (sx, sy) = match degrees {
                90 => (y, sh - 1 - x),
                180 => (sw - 1 - x, sh - 1 - y),
                270 => (sw - 1 - y, x),
                _ => (x, y),
            };
            let v = small[sy * sw + sx];
            rgba.extend_from_slice(&[v, v, v, 255]);
        }
    }
    Some((ow as u32, oh as u32, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grey_of(w: u32, rgba: &[u8], x: u32, y: u32) -> u8 {
        rgba[((y * w + x) * 4) as usize]
    }

    /// A landscape frame turned for a phone held upright: its top-left
    /// corner ends top-right, and the picture stands portrait.
    #[test]
    fn a_frame_is_turned_upright() {
        // 3 by 2: 1 2 3 / 4 5 6
        let grey = [1, 2, 3, 4, 5, 6];
        let (w, h, px) = preview(3, 2, &grey, 90, false).unwrap();
        assert_eq!((w, h), (2, 3));
        let rows: Vec<Vec<u8>> = (0..h).map(|y| (0..w).map(|x| grey_of(w, &px, x, y)).collect()).collect();
        assert_eq!(rows, vec![vec![4, 1], vec![5, 2], vec![6, 3]]);
        let (w, h, px) = preview(3, 2, &grey, 270, false).unwrap();
        let rows: Vec<Vec<u8>> = (0..h).map(|y| (0..w).map(|x| grey_of(w, &px, x, y)).collect()).collect();
        assert_eq!(rows, vec![vec![3, 6], vec![2, 5], vec![1, 4]]);
        let (w, h, px) = preview(3, 2, &grey, 180, false).unwrap();
        let rows: Vec<Vec<u8>> = (0..h).map(|y| (0..w).map(|x| grey_of(w, &px, x, y)).collect()).collect();
        assert_eq!(rows, vec![vec![6, 5, 4], vec![3, 2, 1]]);
        let (w, h, px) = preview(3, 2, &grey, 0, true).unwrap();
        let rows: Vec<Vec<u8>> = (0..h).map(|y| (0..w).map(|x| grey_of(w, &px, x, y)).collect()).collect();
        assert_eq!(rows, vec![vec![3, 2, 1], vec![6, 5, 4]]);
    }

    /// The camera's 640 by 480 frames are shown at 240 by 320, each pixel
    /// the mean of its 2 by 2 block, opaque.
    #[test]
    fn a_frame_is_shrunk_to_the_preview_s_size() {
        let (w, h) = (640, 480);
        let grey: Vec<u8> = (0..w * h).map(|i| if (i % w) % 2 == 0 { 100 } else { 200 }).collect();
        let (pw, ph, px) = preview(w, h, &grey, 90, false).unwrap();
        assert_eq!((pw, ph), (240, 320));
        assert_eq!(px.len(), 240 * 320 * 4);
        assert!(px.chunks(4).all(|p| p == [150, 150, 150, 255]));
        assert!(preview(0, 480, &grey, 0, false).is_none());
        assert!(preview(640, 480, &grey[..10], 0, false).is_none());
    }

    #[test]
    fn the_turn_follows_the_sensor_and_the_display() {
        // A back camera mounted at 90 (most phones), phone upright.
        assert_eq!(turn(90, 0, false), 90);
        // Phone turned left (display rotation 90): the frame is already level.
        assert_eq!(turn(90, 90, false), 0);
        assert_eq!(turn(90, 270, false), 180);
        // A front camera at 270, phone upright.
        assert_eq!(turn(270, 0, true), 270);
        assert_eq!(turn(270, 90, true), 0);
    }
}
