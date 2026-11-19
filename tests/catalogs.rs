use std::collections::BTreeSet;
use std::path::Path;

use drv_i18n::{Catalog, Msg, keys, variables};
use papilla::EnrollError;

const BENCH: [(&str, &str); 2] = [("ru", include_str!("../locales/bench/ru.ftl")), ("en", include_str!("../locales/bench/en.ftl"))];

fn literals(dir: &Path, prefix: &str, found: &mut BTreeSet<String>) {
    for entry in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            literals(&path, prefix, found);
        } else if path.extension().is_some_and(|x| x == "rs") {
            let source = std::fs::read_to_string(&path).unwrap();
            let needle = format!("\"{prefix}");
            for (at, _) in source.match_indices(&needle) {
                let byte_string = source[..at].ends_with('b');
                let rest = source[at + needle.len()..].split('"').next().unwrap_or("");
                let is_key = !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
                if is_key && !byte_string {
                    found.insert(format!("{prefix}{rest}"));
                }
            }
        }
    }
}

fn check(sources: &[(&str, &str); 2], prefix: &str) {
    let mut used = BTreeSet::new();
    literals(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), prefix, &mut used);
    assert!(used.len() >= 10, "{used:?}");
    let reference: BTreeSet<String> = keys(sources[0].1).into_iter().collect();
    for (lang, source) in sources {
        let catalog = Catalog::for_languages(sources, &[lang.to_string()]);
        assert!(catalog.is_loaded(), "{lang}: catalogue does not parse");
        let present: BTreeSet<String> = keys(source).into_iter().collect();
        assert_eq!(present, reference, "{lang}: key set differs");
        for key in &used {
            assert!(catalog.has(key), "{lang}: missing {key}");
        }
        for key in &present {
            assert!(used.contains(key), "{lang}: unused {key}");
            assert_eq!(variables(source, key), variables(sources[0].1, key), "{lang}: variables of {key}");
        }
    }
}

#[test]
fn library_messages_are_complete_in_every_language() {
    check(&papilla::LOCALES, "papilla-");
}

#[test]
fn bench_messages_are_complete_in_every_language() {
    check(&BENCH, "bench-");
}

#[test]
fn every_enrollment_error_renders_with_its_own_arguments() {
    let errors = [
        EnrollError::Featureless { points: 3, needed: 16 },
        EnrollError::Misfit,
        EnrollError::Fixed { still: 3, touches: 8 },
        EnrollError::Incomplete { touches: 5, kept: 2 },
        EnrollError::Inconsistent { kept: 2, total: 40 },
    ];
    for language in ["ru", "en"] {
        let catalog = Catalog::for_languages(&papilla::LOCALES, &[language.to_string()]);
        for error in errors {
            let message = error.args().into_iter().fold(Msg::new(error.key()), |message, (name, value)| message.arg(name, value));
            assert!(catalog.try_render(&message).is_ok(), "{language}: {error}");
        }
    }
}
