#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polarity {
    RidgesBright,
    RidgesDark,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Detector {
    pub upsample: bool,
    pub max_keypoints: usize,
    pub contrast_threshold: f32,
    pub edge_threshold: f32,
}

impl Detector {
    pub const SMALL_SENSOR: Detector = Detector { upsample: true, max_keypoints: 512, contrast_threshold: 0.04, edge_threshold: 10.0 };
    pub const LARGE_SENSOR: Detector = Detector { upsample: false, max_keypoints: 768, contrast_threshold: 0.04, edge_threshold: 10.0 };
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matching {
    pub ratio: f32,
    pub length_tolerance: f32,
    pub angle_tolerance: f32,
    pub min_pair_length: f32,
}

impl Matching {
    pub const VALIDATED: Matching = Matching { ratio: 0.75, length_tolerance: 0.05, angle_tolerance: 0.10, min_pair_length: 4.0 };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Enrolling {
    pub min_touches: usize,
    pub max_touches: usize,
    pub streak: usize,
    pub confident_score: usize,
    pub redundant_score: usize,
    pub adapt_limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Profile {
    pub name: &'static str,
    pub revision: u16,
    pub width: usize,
    pub height: usize,
    pub dpi: u16,
    pub polarity: Polarity,
    pub detector: Detector,
    pub matching: Matching,
    pub enrolling: Enrolling,
    pub accept_score: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileError(pub &'static str);

impl ProfileError {
    pub fn key(&self) -> &'static str {
        self.0
    }
}

impl core::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for ProfileError {}

pub const MAX_NAME: usize = 64;
pub const MIN_SIDE: usize = 32;
pub const MAX_SIDE: usize = 1024;
pub const MAX_PIXELS: usize = 1 << 20;
pub const MAX_PIXELS_UPSAMPLED: usize = 1 << 18;
pub const MIN_ACCEPT_SCORE: usize = 16;
pub const MIN_KEYPOINTS: usize = 16;
pub const CONFIDENT_MARGIN: usize = 2;
pub const MAX_TOUCHES: usize = 400;

const fn name_problem(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_NAME || !(bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit()) {
        return true;
    }
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
            return true;
        }
        i += 1;
    }
    false
}

const fn outside(value: f32, low: f32, high: f32) -> bool {
    !(value >= low && value <= high)
}

impl Matching {
    pub const fn usable(&self) -> bool {
        let measured = Matching::VALIDATED;
        !(outside(self.ratio, 0.5, measured.ratio)
            || outside(self.length_tolerance, 0.005, measured.length_tolerance)
            || outside(self.angle_tolerance, 0.01, measured.angle_tolerance)
            || outside(self.min_pair_length, measured.min_pair_length, 64.0))
    }
}

impl Detector {
    pub const fn usable(&self) -> bool {
        let measured = if self.upsample { Detector::SMALL_SENSOR } else { Detector::LARGE_SENSOR };
        !(self.max_keypoints < 32
            || self.max_keypoints > measured.max_keypoints
            || outside(self.contrast_threshold, measured.contrast_threshold, 0.2)
            || outside(self.edge_threshold, 2.0, measured.edge_threshold))
    }
}

impl Profile {
    pub const fn problem(&self) -> Option<&'static str> {
        if name_problem(self.name) {
            return Some("papilla-profile-name");
        }
        if self.revision == 0 {
            return Some("papilla-profile-revision");
        }
        let pixels = self.width.saturating_mul(self.height);
        let limit = if self.detector.upsample { MAX_PIXELS_UPSAMPLED } else { MAX_PIXELS };
        if self.width < MIN_SIDE || self.height < MIN_SIDE || self.width > MAX_SIDE || self.height > MAX_SIDE || pixels > limit {
            return Some("papilla-profile-dimensions");
        }
        if self.dpi < 100 || self.dpi > 2000 {
            return Some("papilla-profile-dpi");
        }
        if !self.detector.usable() {
            return Some("papilla-profile-detector");
        }
        let shorter_side = if self.width < self.height { self.width } else { self.height };
        if !self.matching.usable() || self.matching.min_pair_length * 4.0 > shorter_side as f32 {
            return Some("papilla-profile-matching");
        }
        if self.accept_score < MIN_ACCEPT_SCORE {
            return Some("papilla-profile-accept-score");
        }
        let e = &self.enrolling;
        if e.min_touches < 3
            || e.max_touches < e.min_touches
            || e.max_touches > MAX_TOUCHES
            || e.streak == 0
            || e.streak > e.max_touches
            || e.confident_score < self.accept_score.saturating_mul(CONFIDENT_MARGIN)
            || e.redundant_score <= e.confident_score
            || e.adapt_limit > MAX_TOUCHES
        {
            return Some("papilla-profile-enrolling");
        }
        None
    }

    pub const fn validated(self) -> Self {
        if let Some(problem) = self.problem() {
            panic!("{}", problem);
        }
        self
    }

    pub fn check(&self) -> Result<(), ProfileError> {
        match self.problem() {
            Some(problem) => Err(ProfileError(problem)),
            None => Ok(()),
        }
    }

    pub const fn pixels(&self) -> usize {
        self.width * self.height
    }

    pub const fn max_templates(&self) -> usize {
        self.enrolling.max_touches.saturating_add(self.enrolling.adapt_limit)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const TEST: Profile = Profile {
        name: "test-80x80",
        revision: 1,
        width: 80,
        height: 80,
        dpi: 500,
        polarity: Polarity::RidgesBright,
        detector: Detector::SMALL_SENSOR,
        matching: Matching::VALIDATED,
        enrolling: Enrolling { min_touches: 3, max_touches: 8, streak: 2, confident_score: 40, redundant_score: 300, adapt_limit: 2 },
        accept_score: 20,
    }
    .validated();

    #[test]
    fn sane_profile_passes() {
        assert_eq!(TEST.check(), Ok(()));
        assert_eq!(TEST.max_templates(), 10);
    }

    #[test]
    fn unsafe_values_are_named() {
        let bad = |p: Profile| p.problem();
        assert_eq!(bad(Profile { name: "Bad Name", ..TEST }), Some("papilla-profile-name"));
        assert_eq!(bad(Profile { name: "-x", ..TEST }), Some("papilla-profile-name"));
        assert_eq!(bad(Profile { revision: 0, ..TEST }), Some("papilla-profile-revision"));
        assert_eq!(bad(Profile { width: 16, ..TEST }), Some("papilla-profile-dimensions"));
        assert_eq!(bad(Profile { width: 1024, height: 1024, ..TEST }), Some("papilla-profile-dimensions"));
        assert_eq!(bad(Profile { accept_score: 3, ..TEST }), Some("papilla-profile-accept-score"));
        assert_eq!(bad(Profile { matching: Matching { ratio: 0.99, ..Matching::VALIDATED }, ..TEST }), Some("papilla-profile-matching"));
        assert_eq!(bad(Profile { matching: Matching { angle_tolerance: f32::NAN, ..Matching::VALIDATED }, ..TEST }), Some("papilla-profile-matching"));
        assert_eq!(bad(Profile { detector: Detector { max_keypoints: 100_000, ..Detector::SMALL_SENSOR }, ..TEST }), Some("papilla-profile-detector"));
        let weak = Enrolling { confident_score: 5, ..TEST.enrolling };
        assert_eq!(bad(Profile { enrolling: weak, ..TEST }), Some("papilla-profile-enrolling"));
        let thin = Enrolling { confident_score: TEST.accept_score * CONFIDENT_MARGIN - 1, ..TEST.enrolling };
        assert_eq!(bad(Profile { enrolling: thin, ..TEST }), Some("papilla-profile-enrolling"));
        assert_eq!(bad(Profile { accept_score: MIN_ACCEPT_SCORE - 1, ..TEST }), Some("papilla-profile-accept-score"));
        let unreachable = Matching { min_pair_length: 21.0, ..Matching::VALIDATED };
        assert_eq!(bad(Profile { matching: unreachable, ..TEST }), Some("papilla-profile-matching"));
    }

    #[test]
    fn nothing_looser_than_what_was_measured_passes() {
        let m = Matching::VALIDATED;
        let looser = [
            Matching { ratio: 0.76, ..m },
            Matching { length_tolerance: 0.051, ..m },
            Matching { angle_tolerance: 0.101, ..m },
            Matching { min_pair_length: 3.9, ..m },
        ];
        for matching in looser {
            assert!(!matching.usable(), "{matching:?}");
            assert_eq!(Profile { matching, ..TEST }.problem(), Some("papilla-profile-matching"));
        }
        let stricter = Matching { ratio: 0.7, length_tolerance: 0.03, angle_tolerance: 0.05, min_pair_length: 6.0 };
        assert_eq!(Profile { matching: stricter, ..TEST }.problem(), None);
        let d = Detector::SMALL_SENSOR;
        for detector in [Detector { max_keypoints: 513, ..d }, Detector { contrast_threshold: 0.039, ..d }, Detector { edge_threshold: 10.1, ..d }] {
            assert_eq!(Profile { detector, ..TEST }.problem(), Some("papilla-profile-detector"), "{detector:?}");
        }
        assert_eq!(Profile { detector: Detector { max_keypoints: 64, contrast_threshold: 0.06, edge_threshold: 8.0, ..d }, ..TEST }.problem(), None);
    }

    #[test]
    fn large_sensor_without_upsampling_is_allowed() {
        let large = Profile { name: "large", width: 256, height: 360, detector: Detector::LARGE_SENSOR, ..TEST };
        assert_eq!(large.check(), Ok(()));
        let too_big_upsampled = Profile { detector: Detector { upsample: true, ..Detector::LARGE_SENSOR }, width: 600, height: 600, ..large };
        assert_eq!(too_big_upsampled.problem(), Some("papilla-profile-dimensions"));
    }
}
