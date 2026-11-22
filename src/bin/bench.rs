use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use drv_i18n::{Catalog, Fail, Msg};
use papilla::{Detector, Matching, Template};

type BoxError = Box<dyn std::error::Error>;
type Tally = (Vec<u64>, Vec<(usize, usize)>);

const LOCALES: [(&str, &str); 4] =
    [("ru", include_str!("../../locales/bench/ru.ftl")), ("en", include_str!("../../locales/bench/en.ftl")), papilla::LOCALES[0], papilla::LOCALES[1]];
const SCORES_FILE: &str = "papilla-scores.csv";
const TAIL: [usize; 9] = [4, 6, 8, 10, 12, 14, 16, 20, 24];

struct Row {
    path: String,
    finger: String,
    source: String,
}

struct Bench {
    catalog: Catalog,
    detector: Detector,
    matching: Matching,
}

fn read_pgm(path: &str) -> Result<(usize, usize, Vec<u8>), Fail> {
    let bad = |what: &str| Fail::new("bench-bad-image").arg("path", path).sub("detail", what);
    let data = std::fs::read(path).map_err(|e| Fail::new("bench-bad-image").arg("path", path).arg("detail", e))?;
    let mut fields: Vec<&[u8]> = Vec::new();
    let mut pos = 0;
    while fields.len() < 4 {
        while data.get(pos).is_some_and(u8::is_ascii_whitespace) {
            pos += 1;
        }
        let start = pos;
        while data.get(pos).is_some_and(|b| !b.is_ascii_whitespace()) {
            pos += 1;
        }
        if start == pos {
            return Err(bad("bench-image-header"));
        }
        fields.push(&data[start..pos]);
    }
    let number = |f: &[u8]| std::str::from_utf8(f).ok().and_then(|s| s.parse::<usize>().ok());
    let (Some(w), Some(h)) = (number(fields[1]), number(fields[2])) else { return Err(bad("bench-image-header")) };
    if fields[0] != b"P5" || fields[3] != b"255" {
        return Err(bad("bench-image-format"));
    }
    let pixels = w
        .checked_mul(h)
        .and_then(|n| pos.checked_add(1).and_then(|start| Some(start..start.checked_add(n)?)))
        .and_then(|range| data.get(range))
        .ok_or_else(|| bad("bench-image-size"))?;
    Ok((w, h, pixels.to_vec()))
}

fn quoted(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) { format!("\"{}\"", field.replace('"', "\"\"")) } else { field.to_string() }
}

fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let parts: Vec<Vec<(usize, R)>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        out.push((i, f(item)));
                    }
                    out
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().expect("worker")).collect()
    });
    let mut all: Vec<(usize, R)> = parts.into_iter().flatten().collect();
    all.sort_by_key(|(i, _)| *i);
    all.into_iter().map(|(_, r)| r).collect()
}

fn read_rows(list: &str) -> Result<Vec<Row>, BoxError> {
    let text = std::fs::read_to_string(list).map_err(|e| Fail::new("bench-bad-image").arg("path", list).arg("detail", e))?;
    Ok(text
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() >= 3).then(|| Row { path: f[0].into(), finger: f[1].into(), source: f[2].into() })
        })
        .collect())
}

fn cores() -> f64 {
    std::thread::available_parallelism().map_or(1, |n| n.get()) as f64
}

impl Bench {
    fn say(&self, msg: Msg) {
        println!("{}", self.catalog.render(&msg));
    }

    fn head(&self, key: &str) -> String {
        self.catalog.text(key)
    }

    fn score(&self, a: &Template, b: &Template) -> usize {
        a.score(b, &self.matching)
    }

    fn templates(&self, rows: &[Row]) -> Result<Vec<Template>, BoxError> {
        let started = Instant::now();
        let extracted = parallel_map(rows, |r| {
            let (w, h, px) = read_pgm(&r.path)?;
            let unusable = |e: papilla::FrameError| Fail::new("bench-bad-image").arg("path", &r.path).nest("detail", Msg::new(e.key()).args(e.args()));
            Template::from_gray8(w, h, &px, &self.detector).map_err(unusable)
        });
        let mut out = Vec::with_capacity(rows.len());
        for t in extracted {
            out.push(t?);
        }
        let mut counts: Vec<usize> = out.iter().map(Template::len).collect();
        counts.sort_unstable();
        if !counts.is_empty() {
            self.say(
                Msg::new("bench-templates")
                    .num("frames", rows.len())
                    .num("median", counts[counts.len() / 2])
                    .num("p10", counts[counts.len() / 10])
                    .arg("ms", format!("{:.1}", started.elapsed().as_secs_f64() * 1000.0 * cores() / rows.len() as f64)),
            );
        }
        Ok(out)
    }

    fn verify(&self, enroll: usize, list: &str, foreign: Option<&str>) -> Result<(), BoxError> {
        let mut rows = read_rows(list)?;
        if let Some(extra) = foreign {
            rows.extend(read_rows(extra)?.into_iter().map(|r| Row { source: "foreign".into(), ..r }));
        }
        let templates = self.templates(&rows)?;
        let mut by_finger: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, r) in rows.iter().enumerate() {
            if r.source != "foreign" {
                by_finger.entry(r.finger.clone()).or_default().push(i);
            }
        }
        let galleries: Vec<(String, Vec<usize>)> =
            by_finger.into_iter().filter(|(_, ids)| ids.len() >= enroll.saturating_add(20)).map(|(f, ids)| (f, ids[..enroll].to_vec())).collect();
        let enrolled: HashSet<usize> = galleries.iter().flat_map(|(_, ids)| ids.iter().copied()).collect();
        let probes: Vec<usize> = (0..rows.len()).filter(|i| !enrolled.contains(i)).collect();
        let started = Instant::now();
        let scores: Vec<Vec<usize>> = parallel_map(&probes, |&p| {
            galleries.iter().map(|(_, ids)| ids.iter().map(|&g| self.score(&templates[p], &templates[g])).max().unwrap_or(0)).collect()
        });
        let pairs = probes.len() * galleries.len() * enroll;
        self.say(
            Msg::new("bench-comparisons")
                .num("pairs", pairs)
                .arg("ms", format!("{:.2}", started.elapsed().as_secs_f64() * 1000.0 * cores() / pairs.max(1) as f64)),
        );
        let mut csv = String::from("probe,finger,gallery,genuine,score\n");
        for (k, &p) in probes.iter().enumerate() {
            for (g, (f, _)) in galleries.iter().enumerate() {
                csv.push_str(&format!("{},{},{},{},{}\n", quoted(&rows[p].path), quoted(&rows[p].finger), quoted(f), rows[p].finger == *f, scores[k][g]));
            }
        }
        std::fs::write(SCORES_FILE, csv)?;
        println!(
            "{:16} {:>6} {:>8} {:>10} {:>10} {:>22}",
            self.head("bench-head-finger"),
            self.head("bench-head-genuine"),
            self.head("bench-head-median"),
            self.head("bench-head-own"),
            self.head("bench-head-foreign"),
            self.head("bench-head-miss")
        );
        let mut impostors: Vec<usize> = Vec::new();
        for (g, (f, _)) in galleries.iter().enumerate() {
            let mut genuine = Vec::new();
            let (mut own, mut foreign_max) = (0usize, 0usize);
            for (k, &p) in probes.iter().enumerate() {
                let s = scores[k][g];
                if rows[p].finger == *f {
                    genuine.push(s);
                    continue;
                }
                impostors.push(s);
                if rows[p].source == "foreign" {
                    foreign_max = foreign_max.max(s);
                } else {
                    own = own.max(s);
                }
            }
            if genuine.is_empty() {
                continue;
            }
            genuine.sort_unstable();
            let top = own.max(foreign_max);
            let miss = genuine.iter().filter(|&&s| s <= top).count() as f32 / genuine.len() as f32;
            println!(
                "{:16} {:>6} {:>8} {:>10} {:>10} {:>21.1}%",
                drv_i18n::printable(f),
                genuine.len(),
                genuine[genuine.len() / 2],
                own,
                foreign_max,
                miss * 100.0
            );
        }
        for threshold in TAIL {
            let count = impostors.iter().filter(|&&s| s >= threshold).count();
            self.say(Msg::new("bench-impostor-tail").num("threshold", threshold).num("count", count).num("total", impostors.len()));
        }
        self.say(Msg::new("bench-wrote").arg("file", SCORES_FILE));
        Ok(())
    }

    fn pairs(&self, list: &str, foreign: Option<&str>) -> Result<(), BoxError> {
        const BUCKETS: usize = 65;
        const TOP: usize = 12;
        let mut rows = read_rows(list)?;
        if let Some(extra) = foreign {
            rows.extend(read_rows(extra)?);
        }
        let templates = self.templates(&rows)?;
        let ids: Vec<usize> = (0..rows.len()).collect();
        let total: usize = ids.iter().map(|&i| rows.iter().filter(|r| r.finger != rows[i].finger).count()).sum();
        self.say(Msg::new("bench-pairs-intro").num("pairs", total));
        let started = Instant::now();
        let per_probe: Vec<Tally> = parallel_map(&ids, |&i| {
            let mut histogram = vec![0u64; BUCKETS];
            let mut top: Vec<(usize, usize)> = Vec::new();
            for j in 0..rows.len() {
                if rows[j].finger == rows[i].finger {
                    continue;
                }
                let s = self.score(&templates[i], &templates[j]);
                histogram[s.min(BUCKETS - 1)] += 1;
                if s >= 8 {
                    top.push((s, j));
                }
            }
            top.sort_unstable_by(|a, b| b.cmp(a));
            top.truncate(TOP);
            (histogram, top)
        });
        let mut histogram = vec![0u64; BUCKETS];
        let mut top: Vec<(usize, usize, usize)> = Vec::new();
        for (i, (h, t)) in per_probe.iter().enumerate() {
            for (bucket, count) in histogram.iter_mut().zip(h) {
                *bucket += count;
            }
            top.extend(t.iter().map(|&(s, j)| (s, i, j)));
        }
        top.sort_unstable_by(|a, b| b.cmp(a));
        for threshold in [4, 6, 8, 10, 12, 14, 16, 20, 24, 32, 48, 64] {
            let count: u64 = histogram[threshold.min(BUCKETS - 1)..].iter().sum();
            self.say(Msg::new("bench-pairs-count").num("threshold", threshold).num("count", count));
        }
        for (score, i, j) in top.iter().take(TOP) {
            self.say(Msg::new("bench-pairs-top").num("score", score).arg("probe", &rows[*i].path).arg("other", &rows[*j].path));
        }
        let max = top.first().map_or(0, |t| t.0).max(histogram.iter().rposition(|&c| c > 0).unwrap_or(0).min(7));
        self.say(Msg::new("bench-pairs-done").arg("minutes", format!("{:.1}", started.elapsed().as_secs_f64() / 60.0)).num("max", max));
        Ok(())
    }

    fn audit(&self, list: &str) -> Result<(), BoxError> {
        let rows = read_rows(list)?;
        let templates = self.templates(&rows)?;
        let mut fingers: Vec<String> = rows.iter().map(|r| r.finger.clone()).collect();
        fingers.sort();
        fingers.dedup();
        let ids: Vec<usize> = (0..rows.len()).collect();
        let best: Vec<Vec<(usize, usize)>> = parallel_map(&ids, |&i| {
            fingers
                .iter()
                .map(|f| {
                    (0..rows.len()).filter(|&j| j != i && rows[j].finger == *f).map(|j| (self.score(&templates[i], &templates[j]), j)).max().unwrap_or((0, i))
                })
                .collect()
        });
        let mut suspicious = Vec::new();
        for (i, per_finger) in best.iter().enumerate() {
            let own = fingers.iter().position(|f| *f == rows[i].finger).map_or(0, |k| per_finger[k].0);
            for (k, f) in fingers.iter().enumerate() {
                let (score, j) = per_finger[k];
                if *f != rows[i].finger && score >= 10 && score > own {
                    suspicious.push((score, own, i, f.clone(), j));
                }
            }
        }
        suspicious.sort_by_key(|found| std::cmp::Reverse(found.0));
        self.say(Msg::new("bench-suspicious").num("count", suspicious.len()));
        for (score, own, i, f, j) in &suspicious {
            self.say(
                Msg::new("bench-suspicious-row").num("score", score).num("own", own).arg("path", &rows[*i].path).arg("finger", f).arg("other", &rows[*j].path),
            );
        }
        Ok(())
    }

    fn saturate(&self, list: &str, finger: &str, threshold: usize) -> Result<(), BoxError> {
        const POOL: usize = 120;
        const NEED: usize = 160;
        let rows: Vec<Row> = read_rows(list)?.into_iter().filter(|r| r.finger == finger && r.source.starts_with("dataset/")).collect();
        if rows.len() < NEED {
            return Err(Fail::new("bench-too-few").arg("finger", finger).num("need", NEED).num("have", rows.len()).into());
        }
        let templates = self.templates(&rows)?;
        let pool: Vec<usize> = (0..POOL).collect();
        let held_out: Vec<usize> = (POOL..rows.len()).collect();
        let arrival: Vec<usize> = parallel_map(&pool, |&k| (0..k).map(|j| self.score(&templates[k], &templates[j])).max().unwrap_or(0));
        self.say(Msg::new("bench-arrival").arg("finger", finger));
        for (row, chunk) in arrival.chunks(10).enumerate() {
            println!("  {:3}-{:3}: {:?}", row * 10 + 1, row * 10 + 10, chunk);
        }
        let miss = |n: usize| -> f32 {
            let best: Vec<usize> = parallel_map(&held_out, |&p| (0..n).map(|g| self.score(&templates[p], &templates[g])).max().unwrap_or(0));
            best.iter().filter(|&&b| b < threshold).count() as f32 / best.len() as f32 * 100.0
        };
        println!(
            "\n{:>8} {:>6} {:>6} {:>10} {:>18}",
            self.head("bench-head-window"),
            self.head("bench-head-need"),
            self.head("bench-head-score"),
            self.head("bench-head-stop"),
            self.head("bench-head-miss-one")
        );
        for (window, need, score) in [(10, 10, 48), (10, 10, 32), (10, 9, 32), (10, 8, 32), (10, 9, 24), (12, 10, 24), (15, 13, 24)] {
            let stop = (40..POOL).find(|&n| arrival[n - window..n].iter().filter(|&&a| a >= score).count() >= need).unwrap_or(POOL);
            println!("{window:>8} {need:>6} {score:>6} {stop:>10} {:>17.1}%", miss(stop));
        }
        for n in [40, 60, 80, 100, 120] {
            self.say(Msg::new("bench-fixed").num("count", n).arg("miss", format!("{:.1}", miss(n))));
        }
        Ok(())
    }

    fn adapt(&self, list: &str, finger: &str, threshold: usize, enroll: usize) -> Result<(), BoxError> {
        let all = read_rows(list)?;
        let mut rows: Vec<Row> = all
            .iter()
            .filter(|r| r.finger == finger && r.source.starts_with("dataset/"))
            .map(|r| Row { path: r.path.clone(), finger: r.finger.clone(), source: r.source.clone() })
            .collect();
        let genuine_count = rows.len();
        if genuine_count < enroll.saturating_add(20) {
            return Err(Fail::new("bench-too-few").arg("finger", finger).num("need", enroll.saturating_add(20)).num("have", genuine_count).into());
        }
        rows.extend(all.into_iter().filter(|r| r.finger != finger));
        let templates = self.templates(&rows)?;
        let stream: Vec<usize> = (enroll..genuine_count).collect();
        let impostors: Vec<usize> = (genuine_count..rows.len()).collect();
        self.say(Msg::new("bench-adapt-intro").arg("finger", finger).num("enroll", enroll).num("stream", stream.len()).num("impostors", impostors.len()));
        println!(
            "{:>22} {:>12} {:>12} {:>10} {:>10} {:>12}",
            self.head("bench-head-rule"),
            self.head("bench-head-miss-first"),
            self.head("bench-head-miss-second"),
            self.head("bench-head-added"),
            self.head("bench-head-size"),
            self.head("bench-head-impostor-max")
        );
        let rules: [(Option<usize>, Option<usize>, usize); 5] =
            [(None, None, 0), (Some(32), None, 60), (Some(32), Some(300), 60), (Some(32), Some(100), 60), (Some(48), Some(300), 60)];
        for (low, high, cap) in rules {
            let name = match low {
                None => self.head("bench-rule-none"),
                Some(low) => self.catalog.render(&Msg::new("bench-rule-range").num("low", low).arg("high", high.map_or("none".to_string(), |h| h.to_string()))),
            };
            let mut gallery: Vec<usize> = (0..enroll).collect();
            let mut added = 0;
            let mut outcomes = Vec::new();
            for &p in &stream {
                let all = parallel_map(&gallery, |&g| self.score(&templates[p], &templates[g]));
                let best = all.iter().copied().max().unwrap_or(0);
                let anchor = all[..enroll].iter().copied().max().unwrap_or(0);
                outcomes.push(best >= threshold);
                if low.is_some_and(|l| anchor >= l) && high.is_none_or(|h| best < h) {
                    if gallery.len() - enroll >= cap {
                        gallery.remove(enroll);
                    }
                    gallery.push(p);
                    added += 1;
                }
            }
            let half = outcomes.len() / 2;
            let miss = |s: &[bool]| s.iter().filter(|&&ok| !ok).count() as f32 / s.len().max(1) as f32 * 100.0;
            let impostor_best = parallel_map(&impostors, |&i| gallery.iter().map(|&g| self.score(&templates[i], &templates[g])).max().unwrap_or(0))
                .into_iter()
                .max()
                .unwrap_or(0);
            println!("{name:>22} {:>11.1}% {:>11.1}% {added:>10} {:>10} {impostor_best:>12}", miss(&outcomes[..half]), miss(&outcomes[half..]), gallery.len());
        }
        Ok(())
    }
}

const ACCEPT: usize = 24;

fn run(bench: &Bench, args: &[String]) -> Result<(), BoxError> {
    let number = |i: usize, default: usize| args.get(i).map_or(Ok(default), |t| t.parse::<usize>());
    match args.first().map(String::as_str) {
        Some("verify") if args.len() >= 3 => bench.verify(args[1].parse()?, &args[2], args.get(3).map(String::as_str)),
        Some("audit") if args.len() >= 2 => bench.audit(&args[1]),
        Some("pairs") if args.len() >= 2 => bench.pairs(&args[1], args.get(2).map(String::as_str)),
        Some("saturate") if args.len() >= 3 => bench.saturate(&args[1], &args[2], number(3, ACCEPT)?),
        Some("adapt") if args.len() >= 3 => bench.adapt(&args[1], &args[2], ACCEPT, number(3, 90)?),
        _ => Err(Fail::new("bench-usage").into()),
    }
}

fn main() -> std::process::ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let large = args.iter().position(|a| a == "--large-sensor").map(|i| args.remove(i)).is_some();
    let bench =
        Bench { catalog: Catalog::new(&LOCALES), detector: if large { Detector::LARGE_SENSOR } else { Detector::SMALL_SENSOR }, matching: Matching::VALIDATED };
    match run(&bench, &args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", bench.catalog.render(&drv_i18n::as_msg(e.as_ref(), "bench-error")));
            std::process::ExitCode::from(2)
        }
    }
}
