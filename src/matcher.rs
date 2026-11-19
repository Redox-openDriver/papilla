use std::f32::consts::{PI, TAU};

use zeroize::{Zeroize, Zeroizing};

use crate::frame::{Frame, FrameError};
use crate::profile::{Detector, MAX_PIXELS_UPSAMPLED, Matching, Polarity, Profile};
use crate::sift::{self, DESCRIPTOR_LEN, Keypoint};

const FRINGE: f32 = 1e-4;
const EDGE_CHECKS_PER_ANGLE: usize = 64;
const BLOCK: usize = 16;

#[derive(Clone, PartialEq)]
pub struct Template {
    keypoints: Vec<Keypoint>,
}

impl Drop for Template {
    fn drop(&mut self) {
        self.keypoints.zeroize();
    }
}

impl std::fmt::Debug for Template {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Template").field("keypoints", &self.keypoints.len()).finish()
    }
}

impl Template {
    pub fn extract(profile: &Profile, frame: &Frame) -> Result<Self, FrameError> {
        if (frame.width(), frame.height()) != (profile.width, profile.height) {
            return Err(FrameError::Profile { expected: (profile.width, profile.height), got: (frame.width(), frame.height()) });
        }
        Self::detect(frame, profile.polarity, &profile.detector)
    }

    pub fn from_gray8(width: usize, height: usize, pixels: &[u8], detector: &Detector) -> Result<Self, FrameError> {
        Self::detect(&Frame::gray8(width, height, pixels)?, Polarity::RidgesBright, detector)
    }

    fn detect(frame: &Frame, polarity: Polarity, detector: &Detector) -> Result<Self, FrameError> {
        if !detector.usable() {
            return Err(FrameError::Detector);
        }
        if detector.upsample && frame.width() * frame.height() > MAX_PIXELS_UPSAMPLED {
            return Err(FrameError::Dimensions { width: frame.width(), height: frame.height() });
        }
        Ok(Template { keypoints: sift::detect(&frame.normalized(polarity), detector, |x, y| frame.covers(x, y)) })
    }

    pub fn fits(&self, profile: &Profile) -> bool {
        let inside = |k: &Keypoint| k.x >= 0.0 && k.y >= 0.0 && k.x < profile.width as f32 && k.y < profile.height as f32;
        self.keypoints.len() <= profile.detector.max_keypoints && self.keypoints.iter().all(inside)
    }

    pub(crate) fn from_keypoints(mut keypoints: Vec<Keypoint>) -> Self {
        distinct(&mut keypoints);
        Template { keypoints }
    }

    pub fn keypoints(&self) -> &[Keypoint] {
        &self.keypoints
    }

    pub fn len(&self) -> usize {
        self.keypoints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keypoints.is_empty()
    }

    pub fn score(&self, other: &Template, matching: &Matching) -> usize {
        score(&self.keypoints, &other.keypoints, matching)
    }
}

fn distinct(keypoints: &mut Vec<Keypoint>) {
    let place = |k: &Keypoint| (k.x.to_bits(), k.y.to_bits(), k.size.to_bits(), k.angle.to_bits());
    let mut order: Zeroizing<Vec<usize>> = Zeroizing::new((0..keypoints.len()).collect());
    order.sort_unstable_by_key(|&index| (place(&keypoints[index]), index));
    let mut repeated = Zeroizing::new(vec![false; keypoints.len()]);
    for pair in order.windows(2) {
        repeated[pair[1]] = place(&keypoints[pair[0]]) == place(&keypoints[pair[1]]);
    }
    let mut index = 0;
    keypoints.retain_mut(|k| {
        let fresh = !repeated[index];
        index += 1;
        if !fresh {
            k.zeroize();
        }
        fresh
    });
}

fn distance_below(a: &[u8; DESCRIPTOR_LEN], b: &[u8; DESCRIPTOR_LEN], bound: u32) -> u32 {
    let mut sum = 0u32;
    for (ca, cb) in a.as_chunks::<BLOCK>().0.iter().zip(b.as_chunks::<BLOCK>().0) {
        for (&x, &y) in ca.iter().zip(cb) {
            let d = i32::from(x) - i32::from(y);
            sum += (d * d) as u32;
        }
        if sum >= bound {
            return sum;
        }
    }
    sum
}

const UNCLAIMED: [usize; 2] = [usize::MAX; 2];

fn matches(a: &[Keypoint], b: &[Keypoint], ratio: f32) -> Zeroizing<Vec<[f32; 4]>> {
    let mut claimed: Zeroizing<Vec<[usize; 2]>> = Zeroizing::new(vec![UNCLAIMED; b.len()]);
    for (i, ka) in a.iter().enumerate() {
        let (mut best, mut second, mut best_j) = (u32::MAX, u32::MAX, usize::MAX);
        for (j, kb) in b.iter().enumerate() {
            let d = distance_below(&ka.descriptor, &kb.descriptor, second);
            if d < best {
                second = best;
                best = d;
                best_j = j;
            } else if d < second {
                second = d;
            }
        }
        let survives = best_j != usize::MAX && second < u32::MAX && (best as f32) < ratio * ratio * second as f32;
        if survives && (claimed[best_j] == UNCLAIMED || (best as usize) < claimed[best_j][0]) {
            claimed[best_j] = [best as usize, i];
        }
    }
    let mut pairs: Zeroizing<Vec<[usize; 2]>> = Zeroizing::new(Vec::with_capacity(b.len()));
    pairs.extend(claimed.iter().enumerate().filter(|(_, claim)| **claim != UNCLAIMED).map(|(j, claim)| [claim[1], j]));
    pairs.sort_unstable();
    let mut matched = Zeroizing::new(Vec::with_capacity(pairs.len()));
    matched.extend(pairs.iter().map(|&[i, j]| [a[i].x, a[i].y, b[j].x, b[j].y]));
    matched
}

fn same_turn(a: f32, b: f32, tolerance: f32) -> bool {
    ((a - b + PI).rem_euclid(TAU) - PI).abs() <= tolerance
}

fn largest_cluster(angles: &mut [f32], tolerance: f32) -> usize {
    let n = angles.len();
    angles.sort_unstable_by(f32::total_cmp);
    let mut ring = Zeroizing::new(Vec::with_capacity(3 * n));
    ring.extend(angles.iter().map(|a| a - TAU));
    ring.extend_from_slice(angles);
    ring.extend(angles.iter().map(|a| a + TAU));
    let first_at_least = |v: f32| ring.partition_point(|&r| r < v);
    let first_above = |v: f32| ring.partition_point(|&r| r <= v);
    let mut edge_checks = EDGE_CHECKS_PER_ANGLE.saturating_mul(n);
    let mut best = 0;
    for &centre in angles.iter() {
        let outer_low = first_at_least(centre - tolerance - FRINGE);
        let inner_low = first_at_least(centre - tolerance + FRINGE);
        let inner_high = first_above(centre + tolerance - FRINGE).max(inner_low);
        let outer_high = first_above(centre + tolerance + FRINGE).max(inner_high);
        let fringe = (outer_low..inner_low).chain(inner_high..outer_high).take(edge_checks);
        edge_checks -= ((inner_low - outer_low) + (outer_high - inner_high)).min(edge_checks);
        let count = (inner_high - inner_low) + fringe.filter(|&j| same_turn(centre, angles[j % n], tolerance)).count();
        best = best.max(count);
    }
    best
}

pub const UNMOVED: f32 = 4.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Likeness {
    pub moved: usize,
    pub still: usize,
}

impl zeroize::DefaultIsZeroes for Likeness {}

fn displaced(pair: &[f32; 4]) -> bool {
    (pair[0] - pair[2]).hypot(pair[1] - pair[3]) >= UNMOVED
}

fn only(m: &[[f32; 4]], wanted: bool) -> Zeroizing<Vec<[f32; 4]>> {
    let mut chosen = Zeroizing::new(Vec::with_capacity(m.len()));
    chosen.extend(m.iter().filter(|pair| displaced(pair) == wanted));
    chosen
}

pub fn score(a: &[Keypoint], b: &[Keypoint], matching: &Matching) -> usize {
    if a.len() < 2 || b.len() < 2 || !matching.usable() {
        return 0;
    }
    consistent(&only(&matches(a, b, matching.ratio), true), matching)
}

pub(crate) fn likeness(a: &[Keypoint], b: &[Keypoint], matching: &Matching) -> Likeness {
    if a.len() < 2 || b.len() < 2 || !matching.usable() {
        return Likeness::default();
    }
    let m = matches(a, b, matching.ratio);
    Likeness { moved: consistent(&only(&m, true), matching), still: consistent(&only(&m, false), matching) }
}

fn consistent(m: &[[f32; 4]], matching: &Matching) -> usize {
    if m.len() < 3 {
        return 0;
    }
    let mut angles: Zeroizing<Vec<f32>> = Zeroizing::new(Vec::with_capacity(m.len() * (m.len() - 1) / 2));
    for j in 0..m.len() {
        for k in j + 1..m.len() {
            let v1 = (m[j][0] - m[k][0], m[j][1] - m[k][1]);
            let v2 = (m[j][2] - m[k][2], m[j][3] - m[k][3]);
            let l1 = v1.0.hypot(v1.1);
            let l2 = v2.0.hypot(v2.1);
            let (short, long) = (l1.min(l2), l1.max(l2).max(1e-6));
            let turn = (v1.0 * v2.1 - v1.1 * v2.0).atan2(v1.0 * v2.0 + v1.1 * v2.1);
            if !(l1.is_finite() && l2.is_finite() && turn.is_finite()) || short <= matching.min_pair_length || 1.0 - short / long > matching.length_tolerance {
                continue;
            }
            angles.push(turn);
        }
    }
    if angles.len() < 2 {
        return 0;
    }
    largest_cluster(&mut angles, matching.angle_tolerance).saturating_sub(1)
}

pub(crate) fn scores(probe: &Template, templates: &[Template], matching: &Matching) -> Zeroizing<Vec<usize>> {
    against_each(templates, |known| probe.score(known, matching))
}

pub(crate) fn likenesses(probe: &Template, templates: &[Template], matching: &Matching) -> Zeroizing<Vec<Likeness>> {
    against_each(templates, |known| likeness(&probe.keypoints, &known.keypoints, matching))
}

fn against_each<R: Send + zeroize::DefaultIsZeroes>(templates: &[Template], compare: impl Fn(&Template) -> R + Sync) -> Zeroizing<Vec<R>> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(templates.len()).max(1);
    let score_all = |part: &[Template]| {
        let mut part_scores = Zeroizing::new(Vec::with_capacity(part.len()));
        part_scores.extend(part.iter().map(&compare));
        part_scores
    };
    if threads == 1 {
        return score_all(templates);
    }
    let chunk = templates.len().div_ceil(threads);
    std::thread::scope(|scope| {
        let workers: Vec<_> =
            templates.chunks(chunk).map(|part| (part, std::thread::Builder::new().spawn_scoped(scope, move || score_all(part)).ok())).collect();
        let mut all = Zeroizing::new(Vec::with_capacity(templates.len()));
        for (part, worker) in workers {
            let part_scores = match worker {
                Some(handle) => handle.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                None => score_all(part),
            };
            all.extend_from_slice(&part_scores);
        }
        all
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::sift::Plane;

    const M: Matching = Matching::VALIDATED;

    pub(crate) fn texture(seed: u32) -> Vec<u8> {
        let mut state = seed;
        let mut noise = vec![0f32; 80 * 80];
        for v in noise.iter_mut() {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *v = (state >> 8) as f32 / (1u32 << 24) as f32;
        }
        let plane = Plane { width: 80, height: 80, data: noise }.blurred(2.0);
        let lo = plane.data.iter().copied().fold(f32::MAX, f32::min);
        let hi = plane.data.iter().copied().fold(f32::MIN, f32::max);
        plane.data.iter().map(|v| ((v - lo) / (hi - lo) * 255.0) as u8).collect()
    }

    pub(crate) fn template(seed: u32) -> Template {
        Template::from_gray8(80, 80, &texture(seed), &Detector::SMALL_SENSOR).unwrap()
    }

    pub(crate) fn placed(seed: u32, step: usize) -> Template {
        let (dx, dy) = ((step % 4 * 5) as f32, (step / 4 % 4 * 5) as f32);
        let moved = |k: &Keypoint| Keypoint { x: k.x + dx, y: k.y + dy, ..k.clone() };
        Template::from_keypoints(template(seed).keypoints().iter().map(moved).filter(|k| k.x < 80.0 && k.y < 80.0).collect())
    }

    fn rotate90(p: &[u8]) -> Vec<u8> {
        (0..80 * 80).map(|i| p[(79 - i % 80) * 80 + i / 80]).collect()
    }

    fn naive_cluster(angles: &[f32], tolerance: f32) -> usize {
        angles.iter().map(|&a| angles.iter().filter(|&&b| same_turn(a, b, tolerance)).count()).max().unwrap_or(0)
    }

    fn naive_distance(a: &[u8; DESCRIPTOR_LEN], b: &[u8; DESCRIPTOR_LEN]) -> u32 {
        a.iter().zip(b).map(|(&x, &y)| (i32::from(x) - i32::from(y)).pow(2) as u32).sum()
    }

    #[test]
    fn same_texture_rotated_matches_and_other_does_not() {
        let a = texture(7);
        let ta = Template::from_gray8(80, 80, &a, &Detector::SMALL_SENSOR).unwrap();
        let same = Template::from_gray8(80, 80, &rotate90(&a), &Detector::SMALL_SENSOR).unwrap();
        let (s, o) = (ta.score(&same, &M), ta.score(&template(99), &M));
        assert!(s >= 20 && o < 5, "same {s} other {o}");
    }

    #[test]
    fn cluster_search_equals_the_quadratic_definition() {
        let mut state = 12345u32;
        let mut next = || {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 8) as f32 / (1u32 << 24) as f32
        };
        for round in 0..200 {
            let n = 2 + (round * 7) % 150;
            let mut angles: Vec<f32> = (0..n)
                .map(|i| match (round + i) % 5 {
                    0 => PI - 0.05 * next(),
                    1 => -PI + 0.05 * next(),
                    2 => 0.3 + 0.1 * (i % 3) as f32,
                    3 => 0.3 + 0.2 * next(),
                    _ => (next() * 2.0 - 1.0) * PI,
                })
                .collect();
            for tolerance in [0.01, 0.1, 0.3] {
                let expected = naive_cluster(&angles, tolerance);
                assert_eq!(largest_cluster(&mut angles, tolerance), expected, "round {round} tolerance {tolerance}");
            }
        }
    }

    #[test]
    fn bounded_distance_never_changes_the_nearest_neighbours() {
        let (a, b) = (template(1), template(2));
        for ka in a.keypoints() {
            let mut exact: Vec<u32> = b.keypoints().iter().map(|kb| naive_distance(&ka.descriptor, &kb.descriptor)).collect();
            exact.sort_unstable();
            let (mut best, mut second) = (u32::MAX, u32::MAX);
            for kb in b.keypoints() {
                let d = distance_below(&ka.descriptor, &kb.descriptor, second);
                if d < best {
                    second = best;
                    best = d;
                } else if d < second {
                    second = d;
                }
            }
            assert_eq!((best, second), (exact[0], exact[1]));
        }
    }

    fn point(x: f32, y: f32, kind: u8) -> Keypoint {
        let mut descriptor = [0u8; DESCRIPTOR_LEN];
        descriptor[usize::from(kind) % DESCRIPTOR_LEN] = 200;
        descriptor[(usize::from(kind) * 7 + 3) % DESCRIPTOR_LEN] = 90;
        Keypoint { x, y, size: 4.0, angle: 0.0, response: 1.0, descriptor }
    }

    #[test]
    fn repeating_one_pattern_does_not_buy_a_score() {
        let gallery: Vec<Keypoint> = (0..12).map(|i| point(10.0 + 5.0 * f32::from(i), 20.0 + 3.0 * f32::from(i % 4), i)).collect();
        for copies in [3u8, 25, 49, 200] {
            let probe: Vec<Keypoint> =
                (0..copies).flat_map(|n| [point(12.0 + f32::from(n) * 0.25, 8.0, 0), point(48.0 + f32::from(n) * 0.25, 8.0, 1)]).collect();
            assert!(score(&probe, &gallery, &M) <= 1, "{copies} copies scored {}", score(&probe, &gallery, &M));
        }
        let honest: Vec<Keypoint> = gallery.iter().map(|k| Keypoint { x: k.x + 7.0, y: k.y + 2.0, ..k.clone() }).collect();
        assert_eq!(score(&honest, &gallery, &M), 65);
        assert_eq!(likeness(&honest, &gallery, &M), Likeness { moved: 65, still: 0 });
    }

    #[test]
    fn points_that_did_not_move_are_no_evidence() {
        let gallery: Vec<Keypoint> = (0..12).map(|i| point(10.0 + 5.0 * f32::from(i), 20.0 + 3.0 * f32::from(i % 4), i)).collect();
        assert_eq!(likeness(&gallery, &gallery, &M), Likeness { moved: 0, still: 65 });
        assert_eq!(score(&gallery, &gallery, &M), 0);
        let nudged: Vec<Keypoint> = gallery.iter().map(|k| Keypoint { x: k.x + 0.6, y: k.y - 0.6, ..k.clone() }).collect();
        assert_eq!(likeness(&nudged, &gallery, &M), Likeness { moved: 0, still: 65 });
        let close: Vec<Keypoint> = gallery.iter().map(|k| Keypoint { x: k.x + UNMOVED - 0.1, ..k.clone() }).collect();
        assert_eq!(likeness(&close, &gallery, &M), Likeness { moved: 0, still: 65 });
        let stepped: Vec<Keypoint> = gallery.iter().map(|k| Keypoint { x: k.x + UNMOVED, ..k.clone() }).collect();
        assert_eq!(likeness(&stepped, &gallery, &M), Likeness { moved: 65, still: 0 });
        let mixed: Vec<Keypoint> = gallery.iter().enumerate().map(|(i, k)| Keypoint { x: k.x + if i < 8 { 6.0 } else { 0.0 }, ..k.clone() }).collect();
        assert_eq!(likeness(&mixed, &gallery, &M), Likeness { moved: 27, still: 5 });
        let lost = Keypoint { x: f32::NAN, ..gallery[0].clone() };
        assert_eq!(score(&[lost, gallery[1].clone(), gallery[2].clone()], &gallery, &M), 0);
        let honest: Vec<Keypoint> = gallery.iter().map(|k| Keypoint { x: k.x + 7.0, y: k.y + 2.0, ..k.clone() }).collect();
        let far_away: Vec<Keypoint> =
            honest.iter().enumerate().map(|(i, k)| if i < 6 { Keypoint { x: f32::INFINITY, y: f32::NEG_INFINITY, ..k.clone() } } else { k.clone() }).collect();
        assert_eq!(score(&far_away, &gallery, &M), 14);
    }

    #[test]
    fn every_gallery_point_is_matched_at_most_once_and_by_its_closest_probe_point() {
        let gallery = vec![point(10.0, 10.0, 0), point(50.0, 10.0, 1), point(30.0, 60.0, 2)];
        let mut near = point(11.0, 11.0, 0);
        near.descriptor[5] = 9;
        let probe = vec![near, point(12.0, 12.0, 0), point(51.0, 11.0, 1), point(13.0, 13.0, 0)];
        let found = matches(&probe, &gallery, M.ratio);
        assert_eq!(*found, vec![[12.0, 12.0, 10.0, 10.0], [51.0, 11.0, 50.0, 10.0]]);
    }

    #[test]
    fn exact_duplicate_keypoints_are_dropped() {
        for seed in 1..6 {
            let extracted = template(seed);
            let mut seen = std::collections::HashSet::new();
            assert!(extracted.keypoints().iter().all(|k| seen.insert((k.x.to_bits(), k.y.to_bits(), k.size.to_bits(), k.angle.to_bits()))));
        }
        let original = template(1);
        let doubled: Vec<Keypoint> = original.keypoints().iter().flat_map(|k| [k.clone(), k.clone()]).collect();
        let loaded = Template::from_keypoints(doubled);
        assert_eq!(loaded.keypoints(), original.keypoints());
        assert_eq!(loaded.score(&placed(1, 1), &M), original.score(&placed(1, 1), &M));
    }

    #[test]
    fn unusable_matching_parameters_never_produce_a_score() {
        let (a, b) = (template(1), placed(1, 1));
        assert!(a.score(&b, &M) > 100);
        for broken in [
            Matching { angle_tolerance: f32::NAN, ..M },
            Matching { angle_tolerance: -0.1, ..M },
            Matching { angle_tolerance: 4.0, ..M },
            Matching { ratio: f32::INFINITY, ..M },
            Matching { ratio: 1.5, ..M },
            Matching { length_tolerance: f32::NAN, ..M },
            Matching { min_pair_length: -1.0, ..M },
        ] {
            assert_eq!(a.score(&b, &broken), 0, "{broken:?}");
        }
    }

    #[test]
    fn degenerate_inputs_score_zero() {
        let empty = Template::from_keypoints(Vec::new());
        assert_eq!(empty.score(&template(1), &M), 0);
        assert_eq!(template(1).score(&empty, &M), 0);
        let flat = Template::from_gray8(80, 80, &[9u8; 6400], &Detector::SMALL_SENSOR).unwrap();
        assert!(flat.is_empty());
    }

    #[test]
    fn mask_removes_keypoints_outside_it() {
        let px = texture(5);
        let mut mask = vec![false; 6400];
        for y in 0..80 {
            for x in 0..40 {
                mask[y * 80 + x] = true;
            }
        }
        let frame = Frame::gray8(80, 80, &px).unwrap().masked(&mask).unwrap();
        let t = Template::extract(&crate::profile::tests::TEST, &frame).unwrap();
        assert!(!t.is_empty() && t.keypoints().iter().all(|k| k.x.round() < 40.0));
    }

    #[test]
    fn frame_must_match_the_profile() {
        let px = vec![0u8; 96 * 96];
        let frame = Frame::gray8(96, 96, &px).unwrap();
        assert_eq!(Template::extract(&crate::profile::tests::TEST, &frame).err(), Some(FrameError::Profile { expected: (80, 80), got: (96, 96) }));
    }

    #[test]
    fn a_detector_looser_than_the_measured_one_extracts_nothing() {
        let pixels = texture(1);
        let frame = Frame::gray8(80, 80, &pixels).unwrap();
        let loose = Detector { contrast_threshold: 0.0, ..Detector::SMALL_SENSOR };
        let broken = Detector { edge_threshold: f32::NAN, ..Detector::SMALL_SENSOR };
        for detector in [loose, broken] {
            let profile = Profile { detector, ..crate::profile::tests::TEST };
            assert_eq!(Template::extract(&profile, &frame).err(), Some(FrameError::Detector));
            assert_eq!(Template::from_gray8(80, 80, &pixels, &detector).err(), Some(FrameError::Detector));
        }
        let tighter = Detector { max_keypoints: 32, ..Detector::SMALL_SENSOR };
        assert_eq!(Template::from_gray8(80, 80, &pixels, &tighter).unwrap().len(), 32);
        let (side, started) = (1024, std::time::Instant::now());
        let huge = vec![0u8; side * side];
        let too_large = Some(FrameError::Dimensions { width: side, height: side });
        assert_eq!(Template::from_gray8(side, side, &huge, &Detector::SMALL_SENSOR).err(), too_large);
        let unchecked = Profile { width: side, height: side, ..crate::profile::tests::TEST };
        assert!(unchecked.check().is_err());
        assert_eq!(Template::extract(&unchecked, &Frame::gray8(side, side, &huge).unwrap()).err(), too_large);
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        assert_eq!(format!("{frame:?}"), "Frame { width: 80, height: 80, depth: 8, masked: false }");
    }

    #[test]
    fn parallel_scores_keep_template_order() {
        let probe = placed(3, 1);
        let set: Vec<Template> = (1..=7).map(template).collect();
        let expected: Vec<usize> = set.iter().map(|t| probe.score(t, &M)).collect();
        assert_eq!(*scores(&probe, &set, &M), expected);
        assert!(expected[2] > 100);
    }
}
