use crate::gallery::Gallery;
use crate::matcher::Template;
use crate::profile::Profile;
use zeroize::{Zeroize, Zeroizing};

use crate::sift::{DESCRIPTOR_LEN, Keypoint};

pub const MAGIC: &[u8; 4] = b"PAPL";
pub const VERSION: u8 = 3;
const KEYPOINT_LEN: usize = 5 * 4 + DESCRIPTOR_LEN;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    NotAGallery,
    Version(u8),
    Profile(String),
    Revision { found: u16, expected: u16 },
    Binding,
    Corrupt,
}

impl StorageError {
    pub fn key(&self) -> &'static str {
        match self {
            StorageError::NotAGallery => "papilla-storage-not-a-gallery",
            StorageError::Version(_) => "papilla-storage-version",
            StorageError::Profile(_) => "papilla-storage-profile",
            StorageError::Revision { .. } => "papilla-storage-revision",
            StorageError::Binding => "papilla-storage-binding",
            StorageError::Corrupt => "papilla-storage-corrupt",
        }
    }

    pub fn args(&self) -> Vec<(&'static str, String)> {
        match self {
            StorageError::Version(v) => vec![("version", v.to_string())],
            StorageError::Profile(p) => vec![("profile", p.clone())],
            StorageError::Revision { found, expected } => vec![("found", found.to_string()), ("expected", expected.to_string())],
            _ => Vec::new(),
        }
    }
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.key())?;
        for (name, value) in self.args() {
            write!(f, " {name}={value}")?;
        }
        Ok(())
    }
}

impl std::error::Error for StorageError {}

pub fn encoded_len(gallery: &Gallery, binding: &[u8]) -> usize {
    let fixed = MAGIC.len() + 1 + 1 + gallery.profile().name.len() + 2 + 2 + binding.len() + 4 + 4;
    fixed + gallery.templates().iter().map(|t| 4 + t.len() * KEYPOINT_LEN).sum::<usize>()
}

pub fn encode(gallery: &Gallery, binding: &[u8]) -> Result<Zeroizing<Vec<u8>>, StorageError> {
    let profile = gallery.profile();
    if binding.len() > usize::from(u16::MAX) {
        return Err(StorageError::Binding);
    }
    let mut out = Zeroizing::new(Vec::with_capacity(encoded_len(gallery, binding)));
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.push(profile.name.len() as u8);
    out.extend_from_slice(profile.name.as_bytes());
    out.extend_from_slice(&profile.revision.to_le_bytes());
    out.extend_from_slice(&(binding.len() as u16).to_le_bytes());
    out.extend_from_slice(binding);
    out.extend_from_slice(&(gallery.templates().len() as u32).to_le_bytes());
    out.extend_from_slice(&(gallery.enrolled() as u32).to_le_bytes());
    for t in gallery.templates() {
        out.extend_from_slice(&(t.len() as u32).to_le_bytes());
        for k in t.keypoints() {
            for v in [k.x, k.y, k.size, k.angle, k.response] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&k.descriptor);
        }
    }
    Ok(out)
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], StorageError> {
        let end = self.pos.checked_add(n).ok_or(StorageError::Corrupt)?;
        let slice = self.data.get(self.pos..end).ok_or(StorageError::Corrupt)?;
        self.pos = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], StorageError> {
        self.take(N)?.try_into().map_err(|_| StorageError::Corrupt)
    }

    fn u8(&mut self) -> Result<u8, StorageError> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, StorageError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn count(&mut self) -> Result<usize, StorageError> {
        Ok(u32::from_le_bytes(self.array()?) as usize)
    }

    fn f32(&mut self) -> Result<f32, StorageError> {
        Ok(f32::from_le_bytes(self.array()?))
    }
}

fn keypoint(r: &mut Reader, profile: &Profile) -> Result<Keypoint, StorageError> {
    let (x, y, size, angle, response) = (r.f32()?, r.f32()?, r.f32()?, r.f32()?, r.f32()?);
    let inside = |v: f32, limit: usize| v >= 0.0 && v <= limit as f32;
    let sane = inside(x, profile.width)
        && inside(y, profile.height)
        && size > 0.0
        && size <= (profile.width + profile.height) as f32
        && (0.0..=360.0).contains(&angle)
        && response >= 0.0
        && response.is_finite();
    if !sane {
        return Err(StorageError::Corrupt);
    }
    Ok(Keypoint { x, y, size, angle, response, descriptor: r.array()? })
}

pub fn decode(data: &[u8], profile: Profile, binding: &[u8]) -> Result<Gallery, StorageError> {
    profile.check().map_err(|_| StorageError::Corrupt)?;
    let mut r = Reader { data, pos: 0 };
    if r.take(MAGIC.len()).map_err(|_| StorageError::NotAGallery)? != MAGIC {
        return Err(StorageError::NotAGallery);
    }
    let version = r.u8()?;
    if !(1..=VERSION).contains(&version) {
        return Err(StorageError::Version(version));
    }
    let name_len = usize::from(r.u8()?);
    let name = r.take(name_len)?;
    if name != profile.name.as_bytes() {
        let shown: String =
            String::from_utf8_lossy(name).chars().map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' { c } else { '?' }).collect();
        return Err(StorageError::Profile(shown));
    }
    let revision = if version >= 3 { r.u16()? } else { 1 };
    if revision != profile.revision {
        return Err(StorageError::Revision { found: revision, expected: profile.revision });
    }
    let binding_len = usize::from(r.u16()?);
    if r.take(binding_len)? != binding {
        return Err(StorageError::Binding);
    }
    let count = r.count()?;
    let enrolled = if version >= 2 { r.count()? } else { count };
    let e = &profile.enrolling;
    if count > profile.max_templates() || enrolled > count || enrolled > e.max_touches || count - enrolled > e.adapt_limit {
        return Err(StorageError::Corrupt);
    }
    let mut templates = Vec::with_capacity(count);
    for _ in 0..count {
        let n = r.count()?;
        if n > profile.detector.max_keypoints || n > (data.len() - r.pos) / KEYPOINT_LEN {
            return Err(StorageError::Corrupt);
        }
        let mut keypoints = Vec::with_capacity(n);
        for _ in 0..n {
            match keypoint(&mut r, &profile) {
                Ok(point) => keypoints.push(point),
                Err(e) => {
                    keypoints.zeroize();
                    return Err(e);
                }
            }
        }
        templates.push(Template::from_keypoints(keypoints));
    }
    if r.pos != data.len() {
        return Err(StorageError::Corrupt);
    }
    Gallery::from_parts(profile, templates, enrolled).map_err(|_| StorageError::Corrupt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::tests::template;
    use crate::profile::tests::TEST;

    fn gallery() -> Gallery {
        Gallery::from_parts(TEST, vec![template(1), template(2), template(3)], 2).unwrap()
    }

    fn encoded() -> Vec<u8> {
        encode(&gallery(), b"sensor-a").unwrap().to_vec()
    }

    #[test]
    fn roundtrip_keeps_everything() {
        let back = decode(&encoded(), TEST, b"sensor-a").expect("decode");
        assert_eq!(back, gallery());
        assert_eq!((back.enrolled(), back.adapted()), (2, 1));
    }

    #[test]
    fn other_sensor_profile_or_revision_is_refused() {
        let bytes = encoded();
        assert_eq!(decode(&bytes, TEST, b"sensor-b").err(), Some(StorageError::Binding));
        assert_eq!(decode(&bytes, Profile { name: "other", ..TEST }, b"sensor-a").err(), Some(StorageError::Profile("test-80x80".into())));
        assert_eq!(decode(&bytes, Profile { revision: 2, ..TEST }, b"sensor-a").err(), Some(StorageError::Revision { found: 1, expected: 2 }));
        assert_eq!(decode(b"nope", TEST, b"sensor-a").err(), Some(StorageError::NotAGallery));
        assert_eq!(decode(b"PAPL\x09", TEST, b"sensor-a").err(), Some(StorageError::Version(9)));
    }

    #[test]
    fn every_truncation_and_trailing_garbage_is_rejected() {
        let bytes = encoded();
        for cut in (0..bytes.len()).step_by(97).chain(bytes.len() - 3..bytes.len()) {
            assert!(decode(&bytes[..cut], TEST, b"sensor-a").is_err(), "cut at {cut}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(decode(&longer, TEST, b"sensor-a").err(), Some(StorageError::Corrupt));
    }

    #[test]
    fn limits_of_the_profile_are_enforced() {
        let small = Profile { enrolling: crate::Enrolling { max_touches: 3, min_touches: 3, streak: 1, adapt_limit: 0, ..TEST.enrolling }, ..TEST };
        assert_eq!(decode(&encoded(), small, b"sensor-a").err(), Some(StorageError::Corrupt));
        let few = Profile { detector: crate::Detector { max_keypoints: 32, ..TEST.detector }, ..TEST };
        assert_eq!(decode(&encoded(), few, b"sensor-a").err(), Some(StorageError::Corrupt));
    }

    #[test]
    fn hostile_numbers_are_rejected_without_panicking() {
        let bytes = encoded();
        let header = 4 + 1 + 1 + TEST.name.len() + 2 + 2 + b"sensor-a".len();
        let first_keypoint = header + 4 + 4 + 4;
        for (offset, value) in [(0, f32::NAN), (0, -1.0), (0, 1e9), (4, f32::INFINITY), (8, 0.0), (12, 720.0), (16, f32::NAN)] {
            let mut bad = bytes.clone();
            bad[first_keypoint + offset..first_keypoint + offset + 4].copy_from_slice(&value.to_le_bytes());
            assert_eq!(decode(&bad, TEST, b"sensor-a").err(), Some(StorageError::Corrupt), "offset {offset} value {value}");
        }
        let mut huge = bytes.clone();
        huge[header..header + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&huge, TEST, b"sensor-a").err(), Some(StorageError::Corrupt));
        let mut many = bytes.clone();
        many[header + 8..header + 12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&many, TEST, b"sensor-a").err(), Some(StorageError::Corrupt));
        let mut state = 99u32;
        for _ in 0..300 {
            let mut fuzzed = bytes.clone();
            for _ in 0..8 {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                let at = (state >> 8) as usize % fuzzed.len();
                fuzzed[at] = (state >> 3) as u8;
            }
            let _ = decode(&fuzzed, TEST, b"sensor-a");
        }
    }

    #[test]
    fn older_format_versions_still_load_as_revision_one() {
        let v3 = encoded();
        let name_end = 6 + TEST.name.len();
        let mut v2 = v3[..name_end].to_vec();
        v2[4] = 2;
        v2.extend_from_slice(&v3[name_end + 2..]);
        assert_eq!(decode(&v2, TEST, b"sensor-a").expect("v2"), gallery());
        assert!(matches!(decode(&v2, Profile { revision: 2, ..TEST }, b"sensor-a"), Err(StorageError::Revision { found: 1, expected: 2 })));
    }
}
