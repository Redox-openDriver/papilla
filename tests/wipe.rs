use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use papilla::{Detector, Enrolling, Enrollment, Frame, Matching, Polarity, Profile, Template, decode, encode};

const LARGE: usize = 512;

struct Inspector;

static DIRTY: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static WATCHING: Cell<bool> = const { Cell::new(false) };
}

unsafe impl GlobalAlloc for Inspector {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        if layout.size() >= LARGE && WATCHING.try_with(Cell::get).unwrap_or(false) {
            let left = unsafe { std::slice::from_raw_parts(block, layout.size()) }.iter().filter(|&&byte| byte != 0).count();
            DIRTY.fetch_add(left, Ordering::Relaxed);
        }
        unsafe { System.dealloc(block, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Inspector = Inspector;

static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn bytes_left_behind<T>(work: impl FnOnce() -> T) -> (T, usize) {
    DIRTY.store(0, Ordering::Relaxed);
    WATCHING.with(|watching| watching.set(true));
    let result = work();
    WATCHING.with(|watching| watching.set(false));
    (result, DIRTY.load(Ordering::Relaxed))
}

const SIDE: usize = 80;

const PROFILE: Profile = Profile {
    name: "wipe-test",
    revision: 1,
    width: SIDE,
    height: SIDE,
    dpi: 500,
    polarity: Polarity::RidgesBright,
    detector: Detector::SMALL_SENSOR,
    matching: Matching::VALIDATED,
    enrolling: Enrolling { min_touches: 3, max_touches: 6, streak: 2, confident_score: 48, redundant_score: 300, adapt_limit: 4 },
    accept_score: 24,
};

fn finger(seed: u32, step: usize) -> Vec<u8> {
    let phase = seed as f32 * 0.7;
    (0..SIDE * SIDE)
        .map(|index| {
            let (x, y) = ((index % SIDE + 6 * step) as f32, (index / SIDE + 4 * step) as f32);
            let bend = ((x * 0.07 + phase).sin() * 9.0 + (y * 0.05 - phase).cos() * 7.0) * (1.0 + (x * y * 0.0004).sin());
            let ridge = ((y + bend) * 0.9 + (x * 0.21 + phase).sin() * 2.5).sin();
            let pore = (((x * 12.9898 + y * 78.233 + phase) * 43758.547).sin() * 0.5 + 0.5) * 0.15;
            (((ridge * 0.5 + 0.5) * 0.85 + pore) * 255.0) as u8
        })
        .collect()
}

fn template(seed: u32, step: usize) -> Template {
    let pixels = finger(seed, step);
    Template::extract(&PROFILE, &Frame::gray8(SIDE, SIDE, &pixels).unwrap()).unwrap()
}

#[test]
fn extraction_frees_nothing_that_still_holds_data() {
    let _alone = ONE_AT_A_TIME.lock().unwrap();
    let pixels = finger(1, 0);
    let frame = Frame::gray8(SIDE, SIDE, &pixels).unwrap();
    let (extracted, left) = bytes_left_behind(|| Template::extract(&PROFILE, &frame).unwrap());
    assert!(extracted.len() > 60, "{} keypoints", extracted.len());
    assert_eq!(left, 0, "bytes of intermediate data left in freed memory");
    let crowded = Profile { detector: Detector { max_keypoints: 32, ..Detector::SMALL_SENSOR }, ..PROFILE };
    let (capped, left) = bytes_left_behind(|| Template::extract(&crowded, &frame).unwrap());
    assert_eq!((capped.len(), left), (32, 0));
    let (_, left) = bytes_left_behind(|| drop(capped));
    assert_eq!(left, 0);
}

#[test]
fn comparing_frees_nothing_that_still_holds_data() {
    let _alone = ONE_AT_A_TIME.lock().unwrap();
    let (first, again, copy, other) = (template(1, 0), template(1, 1), template(1, 0), template(5, 0));
    let (same, left) = bytes_left_behind(|| first.score(&again, &PROFILE.matching));
    assert!(same > 60, "{same}");
    assert_eq!(left, 0);
    assert_eq!(bytes_left_behind(|| first.score(&copy, &PROFILE.matching)), (0, 0));
    let (_, left) = bytes_left_behind(|| first.score(&other, &PROFILE.matching));
    assert_eq!(left, 0);
}

#[test]
fn storing_and_loading_free_nothing_that_still_holds_data() {
    let _alone = ONE_AT_A_TIME.lock().unwrap();
    let mut enrollment = Enrollment::new(PROFILE).unwrap();
    for step in 0..3 {
        enrollment.add(template(1, step)).unwrap();
    }
    let (gallery, _) = enrollment.finish().unwrap();
    let (stored, left) = bytes_left_behind(|| encode(&gallery, b"binding").unwrap());
    assert_eq!(left, 0);
    let (loaded, left) = bytes_left_behind(|| decode(&stored, PROFILE, b"binding").unwrap());
    assert_eq!((loaded.enrolled(), left), (3, 0));
    let mut damaged = stored.to_vec();
    let middle = damaged.len() / 2;
    damaged.truncate(middle);
    let (refused, left) = bytes_left_behind(|| decode(&damaged, PROFILE, b"binding").is_err());
    assert!(refused);
    assert_eq!(left, 0);
    let (_, left) = bytes_left_behind(|| drop((stored, loaded, gallery)));
    assert_eq!(left, 0);
}

#[test]
fn scoring_against_a_large_gallery_frees_nothing_that_still_holds_data() {
    let _alone = ONE_AT_A_TIME.lock().unwrap();
    let roomy = Profile { enrolling: Enrolling { min_touches: 40, max_touches: 80, streak: 2, ..PROFILE.enrolling }, ..PROFILE };
    let place = |index: usize| template(1 + (index % 4) as u32, index / 4);
    let mut enrollment = Enrollment::new(roomy).unwrap();
    for index in 0..70 {
        enrollment.add(place(index)).unwrap();
    }
    let next = place(70);
    let (added, left) = bytes_left_behind(|| enrollment.add(next).is_ok());
    assert_eq!((added, left), (true, 0));
    let (mut gallery, _) = enrollment.finish().unwrap();
    assert!(gallery.templates().len() * std::mem::size_of::<usize>() >= LARGE, "{} templates", gallery.templates().len());
    let (probe, another) = (place(73), place(77));
    let (verdict, left) = bytes_left_behind(|| gallery.verify(&probe));
    assert!(verdict.is_match());
    assert_eq!(left, 0);
    let ((verdict, _), left) = bytes_left_behind(|| gallery.verify_adapting(another));
    assert!(verdict.is_match());
    assert_eq!(left, 0);
}
