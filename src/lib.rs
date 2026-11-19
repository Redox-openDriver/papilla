pub mod enroll;
pub mod frame;
pub mod gallery;
pub mod matcher;
pub mod profile;
pub mod sift;
pub mod storage;
#[cfg(any(feature = "aes-256-gcm-siv", feature = "xchacha20-poly1305", feature = "aes-256-gcm"))]
pub mod vault;

pub const LOCALES: [(&str, &str); 2] = [("ru", include_str!("../locales/ru.ftl")), ("en", include_str!("../locales/en.ftl"))];

pub use enroll::{EnrollError, Enrollment, Progress, Summary};
pub use frame::{Frame, FrameError};
pub use gallery::{Gallery, Verdict};
pub use matcher::Template;
pub use profile::{Detector, Enrolling, Matching, Polarity, Profile, ProfileError};
pub use storage::{StorageError, decode, encode};
