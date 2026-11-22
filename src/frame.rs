use crate::profile::{MAX_PIXELS, MAX_SIDE, MIN_SIDE, Polarity};
use crate::sift::Plane;

#[derive(Clone, Copy)]
enum Pixels<'a> {
    Gray8(&'a [u8]),
    Gray16(&'a [u16]),
}

#[derive(Clone, Copy)]
pub struct Frame<'a> {
    width: usize,
    height: usize,
    pixels: Pixels<'a>,
    mask: Option<&'a [bool]>,
}

impl std::fmt::Debug for Frame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let depth = match self.pixels {
            Pixels::Gray8(_) => 8,
            Pixels::Gray16(_) => 16,
        };
        f.debug_struct("Frame").field("width", &self.width).field("height", &self.height).field("depth", &depth).field("masked", &self.mask.is_some()).finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    Dimensions { width: usize, height: usize },
    Size { expected: usize, got: usize },
    Mask { expected: usize, got: usize },
    Profile { expected: (usize, usize), got: (usize, usize) },
    Detector,
}

impl FrameError {
    pub fn key(&self) -> &'static str {
        match self {
            FrameError::Dimensions { .. } => "papilla-frame-dimensions",
            FrameError::Size { .. } => "papilla-frame-size",
            FrameError::Mask { .. } => "papilla-frame-mask",
            FrameError::Profile { .. } => "papilla-frame-profile",
            FrameError::Detector => "papilla-profile-detector",
        }
    }

    pub fn args(&self) -> Vec<(&'static str, String)> {
        match self {
            FrameError::Dimensions { width, height } => vec![("width", width.to_string()), ("height", height.to_string())],
            FrameError::Size { expected, got } | FrameError::Mask { expected, got } => {
                vec![("expected", expected.to_string()), ("got", got.to_string())]
            }
            FrameError::Profile { expected, got } => {
                vec![("expected", format!("{}x{}", expected.0, expected.1)), ("got", format!("{}x{}", got.0, got.1))]
            }
            FrameError::Detector => Vec::new(),
        }
    }
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.key())?;
        for (name, value) in self.args() {
            write!(f, " {name}={value}")?;
        }
        Ok(())
    }
}

impl std::error::Error for FrameError {}

fn check(width: usize, height: usize, len: usize) -> Result<(), FrameError> {
    let sane = (MIN_SIDE..=MAX_SIDE).contains(&width) && (MIN_SIDE..=MAX_SIDE).contains(&height) && width * height <= MAX_PIXELS;
    if !sane {
        return Err(FrameError::Dimensions { width, height });
    }
    if len != width * height {
        return Err(FrameError::Size { expected: width * height, got: len });
    }
    Ok(())
}

impl<'a> Frame<'a> {
    pub fn gray8(width: usize, height: usize, pixels: &'a [u8]) -> Result<Self, FrameError> {
        check(width, height, pixels.len())?;
        Ok(Frame { width, height, pixels: Pixels::Gray8(pixels), mask: None })
    }

    pub fn gray16(width: usize, height: usize, pixels: &'a [u16]) -> Result<Self, FrameError> {
        check(width, height, pixels.len())?;
        Ok(Frame { width, height, pixels: Pixels::Gray16(pixels), mask: None })
    }

    pub fn masked(self, mask: &'a [bool]) -> Result<Self, FrameError> {
        if mask.len() != self.width * self.height {
            return Err(FrameError::Mask { expected: self.width * self.height, got: mask.len() });
        }
        Ok(Frame { mask: Some(mask), ..self })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn covers(&self, x: f32, y: f32) -> bool {
        if !x.is_finite() || !y.is_finite() {
            return false;
        }
        let Some(mask) = self.mask else { return true };
        let (xi, yi) = (x.round(), y.round());
        if xi < 0.0 || yi < 0.0 || xi >= self.width as f32 || yi >= self.height as f32 {
            return false;
        }
        mask[yi as usize * self.width + xi as usize]
    }

    fn level(&self, index: usize, polarity: Polarity) -> u32 {
        match (self.pixels, polarity) {
            (Pixels::Gray8(p), Polarity::RidgesBright) => u32::from(p[index]),
            (Pixels::Gray8(p), Polarity::RidgesDark) => u32::from(u8::MAX - p[index]),
            (Pixels::Gray16(p), Polarity::RidgesBright) => u32::from(p[index]),
            (Pixels::Gray16(p), Polarity::RidgesDark) => u32::from(u16::MAX - p[index]),
        }
    }

    pub fn normalized(&self, polarity: Polarity) -> Plane {
        let count = self.width * self.height;
        let lo = (0..count).map(|index| self.level(index, polarity)).min().unwrap_or(0);
        let hi = (0..count).map(|index| self.level(index, polarity)).max().unwrap_or(0);
        let span = (hi - lo).max(1);
        let data = (0..count).map(|index| ((self.level(index, polarity) - lo) * 255 / span) as f32 / 255.0).collect();
        Plane { width: self.width, height: self.height, data }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_checked() {
        assert_eq!(Frame::gray8(80, 80, &[0; 10]).err(), Some(FrameError::Size { expected: 6400, got: 10 }));
        assert_eq!(Frame::gray8(8, 8, &[0; 64]).err(), Some(FrameError::Dimensions { width: 8, height: 8 }));
        assert_eq!(Frame::gray16(4096, 4096, &[]).err(), Some(FrameError::Dimensions { width: 4096, height: 4096 }));
        let px = [0u8; 6400];
        assert_eq!(Frame::gray8(80, 80, &px).unwrap().masked(&[true; 3]).err(), Some(FrameError::Mask { expected: 6400, got: 3 }));
    }

    #[test]
    fn both_depths_and_polarities_normalise_to_the_same_grid() {
        let mut p8 = [0u8; 6400];
        let mut p16 = [0u16; 6400];
        for i in 0..6400 {
            p8[i] = (i % 200) as u8 + 20;
            p16[i] = ((i % 200) as u16 + 20) * 100;
        }
        let a = Frame::gray8(80, 80, &p8).unwrap().normalized(Polarity::RidgesBright);
        let b = Frame::gray16(80, 80, &p16).unwrap().normalized(Polarity::RidgesBright);
        assert_eq!(a.data, b.data);
        assert_eq!((a.data[0], a.data[199]), (0.0, 1.0));
        let dark = Frame::gray8(80, 80, &p8).unwrap().normalized(Polarity::RidgesDark);
        assert_eq!((dark.data[0], dark.data[199]), (1.0, 0.0));
        let flat = Frame::gray8(80, 80, &[7u8; 6400]).unwrap().normalized(Polarity::RidgesBright);
        assert!(flat.data.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn mask_limits_coverage() {
        let px = [0u8; 6400];
        let mut mask = [false; 6400];
        mask[10 * 80 + 5] = true;
        let f = Frame::gray8(80, 80, &px).unwrap().masked(&mask).unwrap();
        assert!(f.covers(5.2, 9.8) && !f.covers(6.0, 10.0) && !f.covers(-3.0, 10.0) && !f.covers(5.0, 500.0));
        assert!(Frame::gray8(80, 80, &px).unwrap().covers(79.0, 79.0));
        assert!(!f.covers(f32::NAN, 10.0) && !f.covers(5.0, f32::INFINITY) && !Frame::gray8(80, 80, &px).unwrap().covers(f32::NAN, 1.0));
    }
}
