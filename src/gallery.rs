use crate::matcher::{Template, likenesses, scores};
use crate::profile::{Profile, ProfileError};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Match { score: usize, template: usize },
    NoMatch { best: usize },
}

impl Verdict {
    pub fn is_match(&self) -> bool {
        matches!(self, Verdict::Match { .. })
    }

    pub fn score(&self) -> usize {
        match *self {
            Verdict::Match { score, .. } => score,
            Verdict::NoMatch { best } => best,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gallery {
    profile: Profile,
    templates: Vec<Template>,
    enrolled: usize,
}

impl Gallery {
    pub(crate) fn from_parts(profile: Profile, templates: Vec<Template>, enrolled: usize) -> Result<Self, ProfileError> {
        profile.check()?;
        let enrolled = enrolled.min(templates.len());
        Ok(Gallery { profile, templates, enrolled })
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    pub fn templates(&self) -> &[Template] {
        &self.templates
    }

    pub fn enrolled(&self) -> usize {
        self.enrolled
    }

    pub fn adapted(&self) -> usize {
        self.templates.len() - self.enrolled
    }

    fn judge(&self, all: &[usize]) -> Verdict {
        match all.iter().copied().enumerate().max_by_key(|&(i, s)| (s, std::cmp::Reverse(i))) {
            Some((template, score)) if score >= self.profile.accept_score => Verdict::Match { score, template },
            Some((_, best)) => Verdict::NoMatch { best },
            None => Verdict::NoMatch { best: 0 },
        }
    }

    pub fn verify(&self, probe: &Template) -> Verdict {
        if !probe.fits(&self.profile) {
            return Verdict::NoMatch { best: 0 };
        }
        self.judge(&scores(probe, &self.templates, &self.profile.matching))
    }

    pub fn verify_adapting(&mut self, probe: Template) -> (Verdict, bool) {
        if !probe.fits(&self.profile) {
            return (Verdict::NoMatch { best: 0 }, false);
        }
        let alike = likenesses(&probe, &self.templates, &self.profile.matching);
        let all: Zeroizing<Vec<usize>> = Zeroizing::new(alike.iter().map(|like| like.moved).collect());
        let verdict = self.judge(&all);
        let e = &self.profile.enrolling;
        let anchor = all[..self.enrolled].iter().copied().max().unwrap_or(0);
        let twin = alike.iter().any(|like| like.still >= self.profile.accept_score);
        let learn = e.adapt_limit > 0 && anchor >= e.confident_score && verdict.score() < e.redundant_score && !twin;
        if !learn {
            return (verdict, false);
        }
        while self.adapted() >= e.adapt_limit {
            self.templates.remove(self.enrolled);
        }
        self.templates.push(probe);
        (verdict, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::tests::{placed, template};
    use crate::profile::tests::TEST;

    fn gallery(seeds: &[u32]) -> Gallery {
        Gallery::from_parts(TEST, seeds.iter().map(|&s| template(s)).collect(), seeds.len()).unwrap()
    }

    #[test]
    fn accepts_enrolled_and_rejects_stranger() {
        let g = gallery(&[1, 2]);
        assert!(matches!(g.verify(&placed(2, 1)), Verdict::Match { template: 1, .. }));
        let stranger = g.verify(&template(3));
        assert!(!stranger.is_match() && stranger.score() < TEST.accept_score);
    }

    #[test]
    fn a_picture_that_sits_still_under_the_finger_is_not_evidence_for_anyone() {
        use crate::matcher::likeness;
        use crate::sift::Keypoint;
        let stain: Vec<Keypoint> = template(40).keypoints().iter().filter(|k| k.x < 40.0 && k.y < 40.0).cloned().collect();
        let on_a_stained_reader = |seed: u32, step: usize| {
            let finger = placed(seed, step);
            Template::from_keypoints(finger.keypoints().iter().filter(|k| k.x >= 40.0 || k.y >= 40.0).cloned().chain(stain.iter().cloned()).collect())
        };
        let roomy = Profile { enrolling: crate::Enrolling { redundant_score: usize::MAX, ..TEST.enrolling }, ..TEST };
        let mut g = Gallery::from_parts(roomy, (0..3).map(|step| on_a_stained_reader(1, step)).collect(), 3).unwrap();
        let stranger = on_a_stained_reader(2, 5);
        let alike = likeness(stranger.keypoints(), g.templates()[0].keypoints(), &TEST.matching);
        assert!(alike.still >= TEST.accept_score && alike.moved < TEST.accept_score, "{alike:?}");
        assert!(!g.verify(&stranger).is_match());
        let (owner, learned) = g.verify_adapting(on_a_stained_reader(1, 6));
        assert!(owner.is_match() && !learned);
        assert!(!g.verify(&stranger).is_match());
        assert!(!g.verify(&on_a_stained_reader(3, 9)).is_match());
        let mut clean = Gallery::from_parts(roomy, (0..3).map(|step| placed(1, step)).collect(), 3).unwrap();
        let taught: Vec<bool> = (4..8).map(|step| clean.verify_adapting(on_a_stained_reader(1, step)).1).collect();
        assert_eq!(taught, [true, false, false, false]);
        assert!(!clean.verify(&stranger).is_match());
        assert!(!clean.verify(&on_a_stained_reader(3, 9)).is_match());
    }

    #[test]
    fn a_touch_that_almost_repeats_a_stored_one_is_recognised_but_not_learned() {
        let roomy = Profile { enrolling: crate::Enrolling { redundant_score: usize::MAX, ..TEST.enrolling }, ..TEST };
        let mut g = Gallery::from_parts(roomy, vec![placed(1, 0), placed(1, 1)], 2).unwrap();
        let nudged = Template::from_keypoints(placed(1, 1).keypoints().iter().map(|k| crate::sift::Keypoint { x: k.x + 1.0, ..k.clone() }).collect());
        for _ in 0..5 {
            let (verdict, learned) = g.verify_adapting(nudged.clone());
            assert!(matches!(verdict, Verdict::Match { template: 0, .. }) && !learned);
        }
        assert_eq!(g.adapted(), 0);
        let (verdict, learned) = g.verify_adapting(placed(1, 2));
        assert!(verdict.is_match() && learned);
        let (again, learned) = g.verify_adapting(placed(1, 2));
        assert!(again.is_match() && !learned);
        assert_eq!(g.adapted(), 1);
    }

    #[test]
    fn empty_gallery_never_matches() {
        assert_eq!(gallery(&[]).verify(&template(1)), Verdict::NoMatch { best: 0 });
    }

    #[test]
    fn invalid_profile_cannot_make_a_gallery() {
        let weak = Profile { accept_score: 1, ..TEST };
        assert_eq!(Gallery::from_parts(weak, Vec::new(), 0).err(), Some(ProfileError("papilla-profile-accept-score")));
    }

    #[test]
    fn adaptation_only_learns_confident_non_redundant_matches() {
        let mut g = gallery(&[1]);
        let (verdict, learned) = g.verify_adapting(placed(1, 1));
        assert!(verdict.is_match() && verdict.score() >= TEST.enrolling.redundant_score && !learned);
        let (replayed, learned) = g.verify_adapting(template(1));
        assert_eq!((replayed, learned), (Verdict::NoMatch { best: 0 }, false));
        let (verdict, learned) = g.verify_adapting(template(9));
        assert!(!verdict.is_match() && !learned);
        assert_eq!((g.enrolled(), g.adapted()), (1, 0));
    }

    #[test]
    fn a_learned_template_does_not_vouch_for_further_learning() {
        let roomy = Profile { enrolling: crate::Enrolling { redundant_score: usize::MAX, ..TEST.enrolling }, ..TEST };
        let mut g = Gallery::from_parts(roomy, vec![template(1), template(2)], 1).unwrap();
        let (verdict, learned) = g.verify_adapting(placed(2, 1));
        assert!(matches!(verdict, Verdict::Match { template: 1, .. }) && !learned);
        assert_eq!((g.enrolled(), g.adapted()), (1, 1));
        let (verdict, learned) = g.verify_adapting(placed(1, 1));
        assert!(matches!(verdict, Verdict::Match { template: 0, .. }) && learned);
        assert_eq!((g.enrolled(), g.adapted()), (1, 2));
    }

    #[test]
    fn a_probe_that_does_not_fit_the_profile_is_never_learned() {
        let roomy = crate::Enrolling { redundant_score: usize::MAX, ..TEST.enrolling };
        let cramped = Profile { detector: crate::Detector { max_keypoints: 32, ..TEST.detector }, enrolling: roomy, ..TEST };
        let mut g = Gallery::from_parts(cramped, vec![template(1)], 1).unwrap();
        assert!(placed(1, 1).score(&template(1), &cramped.matching) >= cramped.accept_score);
        assert_eq!(g.verify(&placed(1, 1)), Verdict::NoMatch { best: 0 });
        assert_eq!(g.verify_adapting(placed(1, 1)), (Verdict::NoMatch { best: 0 }, false));
        assert_eq!(g.adapted(), 0);
        assert!(crate::decode(&crate::encode(&g, b"binding").unwrap(), cramped, b"binding").is_err());
        let fitting = Gallery::from_parts(Profile { enrolling: roomy, ..TEST }, vec![template(1)], 1).unwrap();
        assert!(crate::decode(&crate::encode(&fitting, b"binding").unwrap(), *fitting.profile(), b"binding").is_ok());
    }

    #[test]
    fn adapted_part_is_a_bounded_queue_and_enrollment_is_kept() {
        let roomy = Profile { enrolling: crate::Enrolling { redundant_score: usize::MAX, ..TEST.enrolling }, ..TEST };
        let mut g = Gallery::from_parts(roomy, vec![template(1)], 1).unwrap();
        for step in 1..6 {
            let (_, learned) = g.verify_adapting(placed(1, step));
            assert!(learned);
        }
        assert_eq!((g.enrolled(), g.adapted()), (1, TEST.enrolling.adapt_limit));
        assert_eq!(g.templates()[0], template(1));
    }
}
