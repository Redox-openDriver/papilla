use crate::gallery::Gallery;
use crate::matcher::{Template, likenesses};
use crate::profile::{MIN_KEYPOINTS, Profile, ProfileError};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub touches: usize,
    pub kept: usize,
    pub best_score: usize,
    pub known: bool,
    pub streak: usize,
    pub complete: bool,
    pub suspect: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub kept: usize,
    pub dropped: usize,
    pub groups: usize,
    pub largest: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollError {
    Featureless { points: usize, needed: usize },
    Misfit,
    Fixed { still: usize, touches: usize },
    Incomplete { touches: usize, kept: usize },
    Inconsistent { kept: usize, total: usize },
    Profile(ProfileError),
}

impl EnrollError {
    pub fn key(&self) -> &'static str {
        match self {
            EnrollError::Featureless { .. } => "papilla-enroll-featureless",
            EnrollError::Misfit => "papilla-enroll-misfit",
            EnrollError::Fixed { .. } => "papilla-enroll-fixed",
            EnrollError::Incomplete { .. } => "papilla-enroll-incomplete",
            EnrollError::Inconsistent { .. } => "papilla-enroll-inconsistent",
            EnrollError::Profile(e) => e.key(),
        }
    }

    pub fn args(&self) -> Vec<(&'static str, String)> {
        match self {
            EnrollError::Featureless { points, needed } => vec![("points", points.to_string()), ("needed", needed.to_string())],
            EnrollError::Fixed { still, touches } => vec![("still", still.to_string()), ("touches", touches.to_string())],
            EnrollError::Incomplete { touches, kept } => vec![("touches", touches.to_string()), ("kept", kept.to_string())],
            EnrollError::Inconsistent { kept, total } => vec![("kept", kept.to_string()), ("total", total.to_string())],
            EnrollError::Misfit | EnrollError::Profile(_) => Vec::new(),
        }
    }
}

impl std::fmt::Display for EnrollError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.key())?;
        for (name, value) in self.args() {
            write!(f, " {name}={value}")?;
        }
        Ok(())
    }
}

impl std::error::Error for EnrollError {}

pub const FIXED_AFTER_TOUCHES: usize = 12;
pub const FIXED_TOUCHES: usize = 3;
pub const FIXED_SHARE: usize = 2;

pub struct Enrollment {
    profile: Profile,
    templates: Vec<Template>,
    partner: Zeroizing<Vec<usize>>,
    group: Zeroizing<Vec<usize>>,
    streak: usize,
    covered: bool,
    still: usize,
}

impl Enrollment {
    pub fn new(profile: Profile) -> Result<Self, ProfileError> {
        profile.check()?;
        let room = profile.enrolling.max_touches;
        let list = || Zeroizing::new(Vec::with_capacity(room));
        Ok(Enrollment { profile, templates: Vec::with_capacity(room), partner: list(), group: list(), streak: 0, covered: false, still: 0 })
    }

    pub fn touches(&self) -> usize {
        self.templates.len()
    }

    pub fn still_touches(&self) -> usize {
        self.still
    }

    fn stuck(&self) -> Result<(), EnrollError> {
        if self.still >= FIXED_TOUCHES { Err(EnrollError::Fixed { still: self.still, touches: self.templates.len() }) } else { Ok(()) }
    }

    pub fn kept(&self) -> usize {
        self.partner.iter().filter(|&&score| score >= self.profile.accept_score).count()
    }

    pub fn is_complete(&self) -> bool {
        self.covered || self.templates.len() >= self.profile.enrolling.max_touches
    }

    fn root(&self, mut member: usize) -> usize {
        while self.group[member] != member {
            member = self.group[member];
        }
        member
    }

    pub fn add(&mut self, template: Template) -> Result<Progress, EnrollError> {
        self.stuck()?;
        if template.len() < MIN_KEYPOINTS {
            return Err(EnrollError::Featureless { points: template.len(), needed: MIN_KEYPOINTS });
        }
        if !template.fits(&self.profile) {
            return Err(EnrollError::Misfit);
        }
        if self.templates.len() >= self.profile.enrolling.max_touches {
            return Ok(Progress {
                touches: self.templates.len(),
                kept: self.kept(),
                best_score: 0,
                known: false,
                streak: self.streak,
                complete: true,
                suspect: false,
            });
        }
        let all = likenesses(&template, &self.templates, &self.profile.matching);
        let accept = self.profile.accept_score;
        let best_score = all.iter().map(|like| like.moved).max().unwrap_or(0);
        let fresh = self.templates.len();
        self.group.push(fresh);
        for (earlier, like) in all.iter().enumerate() {
            self.partner[earlier] = self.partner[earlier].max(like.moved);
            if like.moved >= accept {
                let (a, b) = (self.root(earlier), self.root(fresh));
                self.group[a.max(b)] = a.min(b);
            }
        }
        let known = best_score >= self.profile.enrolling.confident_score;
        self.streak = if known { self.streak + 1 } else { 0 };
        self.templates.push(template);
        self.partner.push(best_score);
        let sharing = all.iter().filter(|like| like.still >= accept).count();
        let suspect = all.len() + 1 >= FIXED_AFTER_TOUCHES && sharing * FIXED_SHARE >= all.len();
        self.still += usize::from(suspect);
        self.stuck()?;
        self.covered |= self.streak >= self.profile.enrolling.streak && self.kept() >= self.profile.enrolling.min_touches;
        Ok(Progress { touches: self.templates.len(), kept: self.kept(), best_score, known, streak: self.streak, complete: self.is_complete(), suspect })
    }

    pub fn finish(&mut self) -> Result<(Gallery, Summary), EnrollError> {
        self.stuck()?;
        let total = self.templates.len();
        let kept = self.kept();
        if !self.is_complete() {
            return Err(EnrollError::Incomplete { touches: total, kept });
        }
        if kept < self.profile.enrolling.min_touches {
            return Err(EnrollError::Inconsistent { kept, total });
        }
        let accept = self.profile.accept_score;
        let mut sizes = Zeroizing::new(vec![0usize; total]);
        for member in (0..total).filter(|&member| self.partner[member] >= accept) {
            sizes[self.root(member)] += 1;
        }
        let summary =
            Summary { kept, dropped: total - kept, groups: sizes.iter().filter(|&&size| size > 0).count(), largest: sizes.iter().copied().max().unwrap_or(0) };
        let partner = std::mem::take(&mut self.partner);
        let templates: Vec<Template> =
            std::mem::take(&mut self.templates).into_iter().zip(partner.iter()).filter(|(_, score)| **score >= accept).map(|(t, _)| t).collect();
        self.group.clear();
        self.streak = 0;
        self.covered = false;
        self.still = 0;
        Gallery::from_parts(self.profile, templates, kept).map(|g| (g, summary)).map_err(EnrollError::Profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::tests::{placed, template};
    use crate::profile::tests::TEST;

    fn touch(e: &mut Enrollment, seed: u32) -> Progress {
        let step = e.touches();
        e.add(placed(seed, step)).expect("usable touch")
    }

    #[test]
    fn stops_after_a_streak_of_known_touches() {
        let mut e = Enrollment::new(TEST).unwrap();
        let first = touch(&mut e, 1);
        assert_eq!((first.touches, first.kept, first.known, first.streak, first.complete), (1, 0, false, 0, false));
        let second = touch(&mut e, 1);
        assert!(second.known && second.streak == 1 && second.kept == 2 && !second.complete);
        let third = touch(&mut e, 1);
        assert!(third.complete && third.streak == 2 && third.kept == 3);
        let (gallery, summary) = e.finish().unwrap();
        assert_eq!((gallery.enrolled(), gallery.adapted(), summary), (3, 0, Summary { kept: 3, dropped: 0, groups: 1, largest: 3 }));
        assert_eq!(e.touches(), 0);
    }

    #[test]
    fn a_stray_touch_is_dropped() {
        let mut e = Enrollment::new(TEST).unwrap();
        for seed in [1, 1, 1, 77, 1, 1] {
            touch(&mut e, seed);
        }
        let (gallery, summary) = e.finish().unwrap();
        assert_eq!(summary, Summary { kept: 5, dropped: 1, groups: 1, largest: 5 });
        assert!(gallery.templates().iter().all(|kept| (0..6).any(|step| *kept == placed(1, step))));
    }

    #[test]
    fn a_streak_alone_does_not_finish_while_too_few_touches_confirm_each_other() {
        let mut e = Enrollment::new(Profile { enrolling: crate::profile::Enrolling { min_touches: 5, ..TEST.enrolling }, ..TEST }).unwrap();
        for seed in [1, 1, 1] {
            touch(&mut e, seed);
        }
        assert!(!e.is_complete());
        assert_eq!(e.finish().err(), Some(EnrollError::Incomplete { touches: 3, kept: 3 }));
        assert_eq!(e.touches(), 3);
        for seed in [50, 1, 51] {
            assert!(!touch(&mut e, seed).complete);
        }
        let fifth = touch(&mut e, 1);
        assert!(!fifth.complete && fifth.kept == 5 && fifth.streak == 1);
        let last = touch(&mut e, 1);
        assert!(last.complete && last.kept == 6);
        let (gallery, summary) = e.finish().unwrap();
        assert_eq!((gallery.enrolled(), summary.dropped), (6, 2));
    }

    #[test]
    fn different_fingers_do_not_enroll_and_the_session_survives_the_refusal() {
        let mut e = Enrollment::new(TEST).unwrap();
        for seed in 1..=6 {
            assert!(!touch(&mut e, seed).known);
        }
        assert_eq!(e.finish().err(), Some(EnrollError::Incomplete { touches: 6, kept: 0 }));
        for seed in 7..=8 {
            touch(&mut e, seed);
        }
        assert!(e.is_complete());
        assert_eq!(e.finish().err(), Some(EnrollError::Inconsistent { kept: 0, total: 8 }));
        assert_eq!(e.touches(), 8);
    }

    #[test]
    fn unlinked_areas_are_reported_as_separate_groups() {
        let mut e = Enrollment::new(TEST).unwrap();
        for seed in [1, 1, 2, 2, 2, 1] {
            touch(&mut e, seed);
        }
        let (_, summary) = e.finish().unwrap();
        assert_eq!(summary, Summary { kept: 6, dropped: 0, groups: 2, largest: 3 });
    }

    #[test]
    fn a_frame_without_features_is_refused_and_not_counted() {
        let mut e = Enrollment::new(TEST).unwrap();
        let blank = Template::from_gray8(80, 80, &[128u8; 6400], &crate::profile::Detector::SMALL_SENSOR).unwrap();
        assert_eq!(e.add(blank).err(), Some(EnrollError::Featureless { points: 0, needed: MIN_KEYPOINTS }));
        assert_eq!(e.touches(), 0);
    }

    #[test]
    fn a_template_that_does_not_fit_the_profile_is_refused() {
        let mut e = Enrollment::new(TEST).unwrap();
        let wide: Vec<u8> = (0..96 * 96).map(|i| ((i * 37 % 251) ^ (i / 96 * 13 % 251)) as u8).collect();
        let larger = Template::from_gray8(96, 96, &wide, &crate::profile::Detector::SMALL_SENSOR).unwrap();
        assert!(larger.len() >= MIN_KEYPOINTS && !larger.fits(&TEST));
        assert_eq!(e.add(larger).err(), Some(EnrollError::Misfit));
        assert_eq!(e.touches(), 0);
        assert!(template(1).fits(&TEST));
        let cramped = Profile { detector: crate::profile::Detector { max_keypoints: 32, ..TEST.detector }, ..TEST };
        assert!(!template(1).fits(&cramped));
    }

    const LONG: Profile = Profile { enrolling: crate::profile::Enrolling { min_touches: 20, max_touches: 20, streak: 20, ..TEST.enrolling }, ..TEST };

    #[test]
    fn a_picture_that_never_moves_is_not_a_finger() {
        let mut e = Enrollment::new(LONG).unwrap();
        let last = FIXED_AFTER_TOUCHES + FIXED_TOUCHES - 1;
        for count in 1..last {
            let progress = e.add(template(1)).expect("too early to tell");
            assert_eq!((progress.touches, progress.kept, progress.known, progress.suspect), (count, 0, false, count >= FIXED_AFTER_TOUCHES));
        }
        let fixed = EnrollError::Fixed { still: FIXED_TOUCHES, touches: last };
        assert_eq!(e.add(template(1)).err(), Some(fixed));
        assert_eq!(e.add(placed(1, 3)).err(), Some(fixed));
        assert_eq!(e.finish().err(), Some(fixed));
        assert_eq!((e.touches(), e.still_touches()), (last, FIXED_TOUCHES));
    }

    #[test]
    fn a_picture_shown_in_two_places_in_turn_is_not_a_finger_either() {
        use crate::sift::Keypoint;
        let shown = |step: usize| {
            let aside = (step % 2) as f32 * crate::matcher::UNMOVED;
            Template::from_keypoints(template(1).keypoints().iter().map(|k| Keypoint { x: k.x + aside, ..k.clone() }).filter(|k| k.x < 80.0).collect())
        };
        let mut e = Enrollment::new(LONG).unwrap();
        let refused = (0..20).find_map(|step| e.add(shown(step)).err());
        assert_eq!(refused, Some(EnrollError::Fixed { still: FIXED_TOUCHES, touches: 17 }));
    }

    #[test]
    fn a_picture_that_stays_put_under_a_moving_finger_stops_the_enrollment_too() {
        use crate::sift::Keypoint;
        let stain: Vec<Keypoint> = template(40).keypoints().iter().filter(|k| k.x < 40.0 && k.y < 40.0).cloned().collect();
        let stained = |seed: u32, step: usize| {
            let finger = placed(seed, step);
            Template::from_keypoints(finger.keypoints().iter().filter(|k| k.x >= 40.0 || k.y >= 40.0).cloned().chain(stain.iter().cloned()).collect())
        };
        let mut over_one_area = Enrollment::new(LONG).unwrap();
        let mut over_two_areas = Enrollment::new(LONG).unwrap();
        let (mut first, mut second) = (None, None);
        for step in 0..20 {
            first = first.or(over_one_area.add(stained(1, step)).err());
            second = second.or(over_two_areas.add(stained(1 + (step % 2) as u32, step)).err());
        }
        let fixed = EnrollError::Fixed { still: FIXED_TOUCHES, touches: FIXED_AFTER_TOUCHES + FIXED_TOUCHES - 1 };
        assert_eq!((first, second), (Some(fixed), Some(fixed)));
        let mut clean = Enrollment::new(LONG).unwrap();
        for step in 0..20 {
            let progress = clean.add(placed(1 + (step % 2) as u32, step)).expect("two areas of a clean finger");
            assert!(!progress.suspect, "touch {step}");
        }
        assert_eq!(clean.still_touches(), 0);
    }

    #[test]
    fn exact_repeats_among_honest_touches_neither_help_nor_stop_the_enrollment() {
        let long = Profile { enrolling: crate::profile::Enrolling { min_touches: 12, max_touches: 12, streak: 12, ..TEST.enrolling }, ..TEST };
        let mut e = Enrollment::new(long).unwrap();
        let first = e.add(placed(1, 0)).unwrap();
        let repeat = e.add(placed(1, 0)).unwrap();
        assert_eq!((first.kept, repeat.kept, repeat.best_score, repeat.known), (0, 0, 0, false));
        for step in [1, 2, 2, 3, 4, 5, 6, 7, 8, 9] {
            e.add(placed(1, step)).expect("a finger that lands a little differently most of the time");
        }
        assert_eq!(e.still_touches(), 0);
        let (gallery, summary) = e.finish().unwrap();
        assert_eq!((gallery.enrolled(), summary.dropped, summary.groups), (12, 0, 1));
    }

    #[test]
    fn once_covered_a_touch_on_a_new_area_does_not_reopen_the_enrollment() {
        let mut e = Enrollment::new(TEST).unwrap();
        for seed in [1, 1, 1] {
            touch(&mut e, seed);
        }
        assert!(e.is_complete());
        let stray = touch(&mut e, 77);
        assert!(stray.complete && stray.streak == 0 && e.is_complete());
        let (gallery, summary) = e.finish().unwrap();
        assert_eq!((gallery.enrolled(), summary.dropped), (3, 1));
        assert!(!e.is_complete());
    }

    #[test]
    fn never_exceeds_the_touch_limit() {
        let mut e = Enrollment::new(TEST).unwrap();
        for seed in 1..=20 {
            touch(&mut e, seed);
        }
        assert_eq!(e.touches(), TEST.enrolling.max_touches);
        assert!(e.is_complete());
    }

    #[test]
    fn rejects_invalid_profiles() {
        assert!(Enrollment::new(Profile { revision: 0, ..TEST }).is_err());
    }
}
