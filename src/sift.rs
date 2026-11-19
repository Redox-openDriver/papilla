pub const LAYERS: usize = 3;
pub const SIGMA: f32 = 1.6;
pub const INITIAL_SIGMA: f32 = 0.5;
pub const BORDER: usize = 5;
pub const MAX_INTERP_STEPS: usize = 5;
pub const ORI_BINS: usize = 36;
pub const ORI_PEAK_RATIO: f32 = 0.8;
pub const ORI_SIG_FACTOR: f32 = 1.5;
pub const ORI_RADIUS: f32 = 3.0 * ORI_SIG_FACTOR;
pub const DESCR_WIDTH: usize = 4;
pub const DESCR_BINS: usize = 8;
pub const DESCR_SCALE: f32 = 3.0;
pub const DESCR_MAG_THRESHOLD: f32 = 0.2;
pub const DESCRIPTOR_LEN: usize = DESCR_WIDTH * DESCR_WIDTH * DESCR_BINS;

use zeroize::{DefaultIsZeroes, Zeroize, Zeroizing};

use crate::profile::Detector;

#[derive(Clone)]
pub struct Plane {
    pub width: usize,
    pub height: usize,
    pub data: Vec<f32>,
}

impl Drop for Plane {
    fn drop(&mut self) {
        self.data.zeroize();
    }
}

impl Plane {
    pub fn new(width: usize, height: usize) -> Self {
        Plane { width, height, data: vec![0.0; width * height] }
    }

    #[inline]
    pub fn at(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.width + x]
    }

    fn reflect(i: isize, n: usize) -> usize {
        let n = n as isize;
        if n == 1 {
            return 0;
        }
        let mut i = i;
        while i < 0 || i >= n {
            i = if i < 0 { -i } else { 2 * n - 2 - i };
        }
        i as usize
    }

    pub fn blurred(&self, sigma: f32) -> Plane {
        if sigma <= 0.0 {
            return self.clone();
        }
        let radius = (sigma * 4.0).round().max(1.0) as isize;
        let mut kernel: Vec<f32> = (-radius..=radius).map(|i| (-((i * i) as f32) / (2.0 * sigma * sigma)).exp()).collect();
        let total: f32 = kernel.iter().sum();
        kernel.iter_mut().for_each(|k| *k /= total);
        let mut tmp = Plane::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                let mut acc = 0.0;
                for (k, w) in kernel.iter().enumerate() {
                    acc += w * self.at(Self::reflect(x as isize + k as isize - radius, self.width), y);
                }
                tmp.data[y * self.width + x] = acc;
            }
        }
        let mut out = Plane::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                let mut acc = 0.0;
                for (k, w) in kernel.iter().enumerate() {
                    acc += w * tmp.at(x, Self::reflect(y as isize + k as isize - radius, self.height));
                }
                out.data[y * self.width + x] = acc;
            }
        }
        out
    }

    pub fn upsampled(&self) -> Plane {
        let mut out = Plane::new(self.width * 2, self.height * 2);
        for y in 0..out.height {
            for x in 0..out.width {
                let sx = ((x as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (self.width - 1) as f32);
                let sy = ((y as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (self.height - 1) as f32);
                let (x0, y0) = (sx.floor() as usize, sy.floor() as usize);
                let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
                let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
                let top = self.at(x0, y0) * (1.0 - fx) + self.at(x1, y0) * fx;
                let bottom = self.at(x0, y1) * (1.0 - fx) + self.at(x1, y1) * fx;
                out.data[y * out.width + x] = top * (1.0 - fy) + bottom * fy;
            }
        }
        out
    }

    pub fn halved(&self) -> Plane {
        let mut out = Plane::new(self.width / 2, self.height / 2);
        for y in 0..out.height {
            for x in 0..out.width {
                out.data[y * out.width + x] = self.at(2 * x, 2 * y);
            }
        }
        out
    }
}

#[derive(Clone, PartialEq)]
pub struct Keypoint {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub angle: f32,
    pub response: f32,
    pub descriptor: [u8; DESCRIPTOR_LEN],
}

impl std::fmt::Debug for Keypoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Keypoint")
    }
}

impl Zeroize for Keypoint {
    fn zeroize(&mut self) {
        self.x.zeroize();
        self.y.zeroize();
        self.size.zeroize();
        self.angle.zeroize();
        self.response.zeroize();
        self.descriptor.zeroize();
    }
}

struct Pyramid {
    gauss: Vec<Vec<Plane>>,
    dog: Vec<Vec<Plane>>,
}

fn build(base: &Plane) -> Pyramid {
    let octaves = ((base.width.min(base.height) as f32).log2() - 2.0).round().max(1.0) as usize;
    let k = 2f32.powf(1.0 / LAYERS as f32);
    let mut sigmas = vec![SIGMA];
    for i in 1..LAYERS + 3 {
        let prev = SIGMA * k.powi(i as i32 - 1);
        let total = prev * k;
        sigmas.push((total * total - prev * prev).sqrt());
    }
    let mut gauss: Vec<Vec<Plane>> = Vec::with_capacity(octaves);
    for o in 0..octaves {
        let mut layers: Vec<Plane> = Vec::with_capacity(LAYERS + 3);
        let first = if o == 0 { base.clone() } else { gauss[o - 1][LAYERS].halved() };
        layers.push(first);
        for i in 1..LAYERS + 3 {
            let next = layers[i - 1].blurred(sigmas[i]);
            layers.push(next);
        }
        gauss.push(layers);
    }
    let dog = gauss
        .iter()
        .map(|layers| {
            layers
                .windows(2)
                .map(|w| Plane { width: w[0].width, height: w[0].height, data: w[1].data.iter().zip(&w[0].data).map(|(a, b)| a - b).collect() })
                .collect()
        })
        .collect();
    Pyramid { gauss, dog }
}

fn is_extremum(dog: &[Plane], layer: usize, x: usize, y: usize, value: f32) -> bool {
    for (near, plane) in dog[layer - 1..=layer + 1].iter().enumerate() {
        for yy in y - 1..=y + 1 {
            for xx in x - 1..=x + 1 {
                if (near, yy, xx) == (1, y, x) {
                    continue;
                }
                let v = plane.at(xx, yy);
                if (value > 0.0 && v > value) || (value < 0.0 && v < value) {
                    return false;
                }
            }
        }
    }
    true
}

struct Refined {
    x: usize,
    y: usize,
    layer: usize,
    offset: [f32; 3],
    contrast: f32,
}

fn solve3(h: [[f32; 3]; 3], b: [f32; 3]) -> Option<[f32; 3]> {
    let det = h[0][0] * (h[1][1] * h[2][2] - h[1][2] * h[2][1]) - h[0][1] * (h[1][0] * h[2][2] - h[1][2] * h[2][0])
        + h[0][2] * (h[1][0] * h[2][1] - h[1][1] * h[2][0]);
    if det.abs() < 1e-12 {
        return None;
    }
    let mut out = [0.0; 3];
    for (col, slot) in out.iter_mut().enumerate() {
        let mut m = h;
        for row in 0..3 {
            m[row][col] = b[row];
        }
        *slot = (m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]))
            / det;
    }
    Some(out)
}

fn refine(dog: &[Plane], layer: usize, x: usize, y: usize, detector: &Detector) -> Option<Refined> {
    let (w, h) = (dog[0].width, dog[0].height);
    let (mut x, mut y, mut layer) = (x, y, layer);
    let mut offset = [0.0f32; 3];
    let mut gradient = [0.0f32; 3];
    let mut converged = false;
    for _ in 0..MAX_INTERP_STEPS {
        let (prev, cur, next) = (&dog[layer - 1], &dog[layer], &dog[layer + 1]);
        let v = cur.at(x, y);
        gradient = [(cur.at(x + 1, y) - cur.at(x - 1, y)) * 0.5, (cur.at(x, y + 1) - cur.at(x, y - 1)) * 0.5, (next.at(x, y) - prev.at(x, y)) * 0.5];
        let dxx = cur.at(x + 1, y) + cur.at(x - 1, y) - 2.0 * v;
        let dyy = cur.at(x, y + 1) + cur.at(x, y - 1) - 2.0 * v;
        let dss = next.at(x, y) + prev.at(x, y) - 2.0 * v;
        let dxy = (cur.at(x + 1, y + 1) - cur.at(x - 1, y + 1) - cur.at(x + 1, y - 1) + cur.at(x - 1, y - 1)) * 0.25;
        let dxs = (next.at(x + 1, y) - next.at(x - 1, y) - prev.at(x + 1, y) + prev.at(x - 1, y)) * 0.25;
        let dys = (next.at(x, y + 1) - next.at(x, y - 1) - prev.at(x, y + 1) + prev.at(x, y - 1)) * 0.25;
        let hessian = [[dxx, dxy, dxs], [dxy, dyy, dys], [dxs, dys, dss]];
        let solved = solve3(hessian, gradient)?;
        offset = [-solved[0], -solved[1], -solved[2]];
        if offset.iter().all(|o| o.abs() < 0.5) {
            converged = true;
            break;
        }
        if offset.iter().any(|o| o.abs() > 1e6) {
            return None;
        }
        let nx = x as isize + offset[0].round() as isize;
        let ny = y as isize + offset[1].round() as isize;
        let nl = layer as isize + offset[2].round() as isize;
        if nl < 1 || nl > LAYERS as isize || nx < BORDER as isize || nx >= (w - BORDER) as isize || ny < BORDER as isize || ny >= (h - BORDER) as isize {
            return None;
        }
        x = nx as usize;
        y = ny as usize;
        layer = nl as usize;
    }
    if !converged {
        return None;
    }
    let cur = &dog[layer];
    let contrast = cur.at(x, y) + 0.5 * (gradient[0] * offset[0] + gradient[1] * offset[1] + gradient[2] * offset[2]);
    if contrast.abs() * (LAYERS as f32) < detector.contrast_threshold {
        return None;
    }
    let v = cur.at(x, y);
    let dxx = cur.at(x + 1, y) + cur.at(x - 1, y) - 2.0 * v;
    let dyy = cur.at(x, y + 1) + cur.at(x, y - 1) - 2.0 * v;
    let dxy = (cur.at(x + 1, y + 1) - cur.at(x - 1, y + 1) - cur.at(x + 1, y - 1) + cur.at(x - 1, y - 1)) * 0.25;
    let trace = dxx + dyy;
    let det = dxx * dyy - dxy * dxy;
    let edge = detector.edge_threshold;
    if det <= 0.0 || trace * trace * edge >= (edge + 1.0).powi(2) * det {
        return None;
    }
    Some(Refined { x, y, layer, offset, contrast })
}

fn orientations(img: &Plane, x: usize, y: usize, scale: f32) -> ([f32; ORI_BINS], usize) {
    let radius = (ORI_RADIUS * scale).round() as isize;
    let sigma = ORI_SIG_FACTOR * scale;
    let mut hist = [0.0f32; ORI_BINS];
    for dy in -radius..=radius {
        let yy = y as isize + dy;
        if yy <= 0 || yy >= img.height as isize - 1 {
            continue;
        }
        for dx in -radius..=radius {
            let xx = x as isize + dx;
            if xx <= 0 || xx >= img.width as isize - 1 {
                continue;
            }
            let (xu, yu) = (xx as usize, yy as usize);
            let gx = img.at(xu + 1, yu) - img.at(xu - 1, yu);
            let gy = img.at(xu, yu - 1) - img.at(xu, yu + 1);
            let weight = (-((dx * dx + dy * dy) as f32) / (2.0 * sigma * sigma)).exp();
            let angle = gy.atan2(gx).to_degrees().rem_euclid(360.0);
            let bin = ((angle * ORI_BINS as f32 / 360.0).round() as usize) % ORI_BINS;
            hist[bin] += weight * (gx * gx + gy * gy).sqrt();
        }
    }
    let n = ORI_BINS;
    let smooth: [f32; ORI_BINS] = std::array::from_fn(|i| {
        (hist[(i + n - 2) % n] + hist[(i + 2) % n]) * (1.0 / 16.0) + (hist[(i + n - 1) % n] + hist[(i + 1) % n]) * (4.0 / 16.0) + hist[i] * (6.0 / 16.0)
    });
    let peak = smooth.iter().copied().fold(0.0f32, f32::max);
    let (mut out, mut found) = ([0.0f32; ORI_BINS], 0);
    for i in 0..n {
        let (l, c, r) = (smooth[(i + n - 1) % n], smooth[i], smooth[(i + 1) % n]);
        if c > l && c > r && c >= ORI_PEAK_RATIO * peak {
            let bin = (i as f32 + 0.5 * (l - r) / (l - 2.0 * c + r)).rem_euclid(n as f32);
            let mut angle = 360.0 - bin * 360.0 / n as f32;
            if (angle - 360.0).abs() < f32::EPSILON {
                angle = 0.0;
            }
            out[found] = angle;
            found += 1;
        }
    }
    (out, found)
}

fn describe(img: &Plane, px: f32, py: f32, angle: f32, scale: f32) -> [u8; DESCRIPTOR_LEN] {
    let d = DESCR_WIDTH as isize;
    let n = DESCR_BINS;
    let ori = 360.0 - angle;
    let (sin_t, cos_t) = ori.to_radians().sin_cos();
    let bins_per_deg = n as f32 / 360.0;
    let exp_scale = -1.0 / (DESCR_WIDTH as f32 * DESCR_WIDTH as f32 * 0.5);
    let hist_width = DESCR_SCALE * scale;
    let max_radius = ((img.width * img.width + img.height * img.height) as f32).sqrt();
    let radius = ((hist_width * std::f32::consts::SQRT_2 * (DESCR_WIDTH as f32 + 1.0) * 0.5).round()).min(max_radius) as isize;
    let (cos_t, sin_t) = (cos_t / hist_width, sin_t / hist_width);
    let (cx, cy) = (px.round() as isize, py.round() as isize);
    let side = DESCR_WIDTH + 2;
    let mut hist = [0.0f32; (DESCR_WIDTH + 2) * (DESCR_WIDTH + 2) * (DESCR_BINS + 2)];
    for i in -radius..=radius {
        for j in -radius..=radius {
            let c_rot = j as f32 * cos_t - i as f32 * sin_t;
            let r_rot = j as f32 * sin_t + i as f32 * cos_t;
            let rbin = r_rot + d as f32 / 2.0 - 0.5;
            let cbin = c_rot + d as f32 / 2.0 - 0.5;
            let (r, c) = (cy + i, cx + j);
            if !(rbin > -1.0 && rbin < d as f32 && cbin > -1.0 && cbin < d as f32) {
                continue;
            }
            if r <= 0 || r >= img.height as isize - 1 || c <= 0 || c >= img.width as isize - 1 {
                continue;
            }
            let (ru, cu) = (r as usize, c as usize);
            let gx = img.at(cu + 1, ru) - img.at(cu - 1, ru);
            let gy = img.at(cu, ru - 1) - img.at(cu, ru + 1);
            let grad_ori = gy.atan2(gx).to_degrees().rem_euclid(360.0);
            let magnitude = (gx * gx + gy * gy).sqrt() * ((c_rot * c_rot + r_rot * r_rot) * exp_scale).exp();
            let obin = ((grad_ori - ori) * bins_per_deg).rem_euclid(n as f32);
            let (r0, c0, o0) = (rbin.floor(), cbin.floor(), obin.floor());
            let (fr, fc, fo) = (rbin - r0, cbin - c0, obin - o0);
            let (r0, c0, o0) = ((r0 as isize + 1) as usize, (c0 as isize + 1) as usize, (o0 as usize) % n);
            for (dr, wr) in [(0, 1.0 - fr), (1, fr)] {
                for (dc, wc) in [(0, 1.0 - fc), (1, fc)] {
                    for (dobin, wo) in [(0, 1.0 - fo), (1, fo)] {
                        let idx = ((r0 + dr) * side + c0 + dc) * (n + 2) + o0 + dobin;
                        hist[idx] += magnitude * wr * wc * wo;
                    }
                }
            }
        }
    }
    let mut out = [0.0f32; DESCRIPTOR_LEN];
    for r in 0..DESCR_WIDTH {
        for c in 0..DESCR_WIDTH {
            let base = ((r + 1) * side + c + 1) * (n + 2);
            hist[base] += hist[base + n];
            hist[base + 1] += hist[base + n + 1];
            for o in 0..n {
                out[(r * DESCR_WIDTH + c) * n + o] = hist[base + o];
            }
        }
    }
    let norm = out.iter().map(|v| v * v).sum::<f32>().sqrt();
    let limit = norm * DESCR_MAG_THRESHOLD;
    out.iter_mut().for_each(|v| *v = v.min(limit));
    let norm = out.iter().map(|v| v * v).sum::<f32>().sqrt().max(f32::EPSILON);
    let descriptor = out.map(|v| (v * 512.0 / norm).round().clamp(0.0, 255.0) as u8);
    hist.zeroize();
    out.zeroize();
    descriptor
}

#[derive(Clone, Copy, Default)]
struct Candidate {
    octave: u32,
    layer: u32,
    x: f32,
    y: f32,
    scale: f32,
    angle: f32,
    response: f32,
}

impl DefaultIsZeroes for Candidate {}

const FIRST_CANDIDATES: usize = 256;

fn keep(candidates: &mut Vec<Candidate>, candidate: Candidate) {
    if candidates.len() == candidates.capacity() {
        let mut larger = Vec::with_capacity((candidates.capacity() * 2).max(FIRST_CANDIDATES));
        larger.extend_from_slice(candidates);
        candidates.zeroize();
        *candidates = larger;
    }
    candidates.push(candidate);
}

fn strongest(candidates: &[Candidate], limit: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    if order.len() > limit {
        order.sort_unstable_by(|&a, &b| candidates[b].response.total_cmp(&candidates[a].response).then(a.cmp(&b)));
        order[limit..].zeroize();
        order.truncate(limit);
        order.sort_unstable();
    }
    order
}

pub fn detect(image: &Plane, detector: &Detector, covers: impl Fn(f32, f32) -> bool) -> Vec<Keypoint> {
    let (base, first_scale) = if detector.upsample {
        let initial = (SIGMA * SIGMA - 4.0 * INITIAL_SIGMA * INITIAL_SIGMA).max(0.01).sqrt();
        (image.upsampled().blurred(initial), 0.5)
    } else {
        let initial = (SIGMA * SIGMA - INITIAL_SIGMA * INITIAL_SIGMA).max(0.01).sqrt();
        (image.blurred(initial), 1.0)
    };
    let pyramid = build(&base);
    let threshold = 0.5 * detector.contrast_threshold / LAYERS as f32;
    let mut candidates: Zeroizing<Vec<Candidate>> = Zeroizing::new(Vec::new());
    for (o, dog) in pyramid.dog.iter().enumerate() {
        let (w, h) = (dog[0].width, dog[0].height);
        if w <= 2 * BORDER || h <= 2 * BORDER {
            continue;
        }
        let mut settled = Zeroizing::new(vec![false; (LAYERS + 1) * w * h]);
        for layer in 1..=LAYERS {
            for y in BORDER..h - BORDER {
                for x in BORDER..w - BORDER {
                    let value = dog[layer].at(x, y);
                    if value.abs() <= threshold || !is_extremum(dog, layer, x, y, value) {
                        continue;
                    }
                    let Some(r) = refine(dog, layer, x, y, detector) else { continue };
                    if std::mem::replace(&mut settled[(r.layer * h + r.y) * w + r.x], true) {
                        continue;
                    }
                    let scale = SIGMA * 2f32.powf((r.layer as f32 + r.offset[2]) / LAYERS as f32);
                    let (ox, oy) = (r.x as f32 + r.offset[0], r.y as f32 + r.offset[1]);
                    let octave_scale = 2f32.powi(o as i32) * first_scale;
                    if !covers(ox * octave_scale, oy * octave_scale) {
                        continue;
                    }
                    let (angles, found) = orientations(&pyramid.gauss[o][r.layer], r.x, r.y, scale);
                    for &angle in &angles[..found] {
                        keep(&mut candidates, Candidate { octave: o as u32, layer: r.layer as u32, x: ox, y: oy, scale, angle, response: r.contrast.abs() });
                    }
                }
            }
        }
    }
    let chosen = Zeroizing::new(strongest(&candidates, detector.max_keypoints));
    let mut keypoints = Zeroizing::new(Vec::with_capacity(chosen.len()));
    for &index in chosen.iter() {
        let found = candidates[index];
        let (octave, layer) = (found.octave as usize, found.layer as usize);
        let octave_scale = 2f32.powi(found.octave as i32) * first_scale;
        keypoints.push(Keypoint {
            x: found.x * octave_scale,
            y: found.y * octave_scale,
            size: found.scale * 2.0 * octave_scale,
            angle: found.angle,
            response: found.response,
            descriptor: describe(&pyramid.gauss[octave][layer], found.x, found.y, found.angle, found.scale),
        });
    }
    std::mem::take(&mut *keypoints)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob_image() -> Plane {
        let mut p = Plane::new(80, 80);
        for y in 0..80 {
            for x in 0..80 {
                let d1 = ((x as f32 - 25.0).powi(2) + (y as f32 - 30.0).powi(2)) / (2.0 * 9.0);
                let d2 = ((x as f32 - 55.0).powi(2) + (y as f32 - 50.0).powi(2)) / (2.0 * 16.0);
                p.data[y * 80 + x] = 0.2 + 0.6 * (-d1).exp() + 0.5 * (-d2).exp();
            }
        }
        p
    }

    const D: Detector = Detector::SMALL_SENSOR;
    const ANYWHERE: fn(f32, f32) -> bool = |_, _| true;

    #[test]
    fn finds_blobs_near_their_centres() {
        let kps = detect(&blob_image(), &D, ANYWHERE);
        assert!(kps.iter().any(|k| (k.x - 25.0).abs() < 2.0 && (k.y - 30.0).abs() < 2.0), "{:?}", kps.iter().map(|k| (k.x, k.y, k.size)).collect::<Vec<_>>());
        assert_eq!(format!("{:?}", kps[0]), "Keypoint");
        assert!(kps.iter().any(|k| (k.x - 55.0).abs() < 2.0 && (k.y - 50.0).abs() < 2.0));
    }

    #[test]
    fn descriptor_survives_rotation() {
        let img = blob_image();
        let mut rotated = Plane::new(80, 80);
        for y in 0..80 {
            for x in 0..80 {
                rotated.data[y * 80 + x] = img.at(79 - y, x);
            }
        }
        let a = detect(&img, &D, ANYWHERE);
        let b = detect(&rotated, &D, ANYWHERE);
        let best = a
            .iter()
            .flat_map(|ka| {
                b.iter().map(move |kb| ka.descriptor.iter().zip(&kb.descriptor).map(|(&p, &q)| (f32::from(p) - f32::from(q)).powi(2)).sum::<f32>().sqrt())
            })
            .fold(f32::MAX, f32::min);
        assert!(best < 100.0, "{best}");
    }

    #[test]
    fn keypoint_limit_keeps_the_strongest_in_scan_order() {
        let all = detect(&blob_image(), &D, ANYWHERE);
        assert!(all.len() >= 2);
        let limited = detect(&blob_image(), &Detector { max_keypoints: 1, ..D }, ANYWHERE);
        let strongest = all.iter().fold(&all[0], |best, k| if k.response > best.response { k } else { best });
        assert_eq!(limited, vec![strongest.clone()]);
    }

    #[test]
    fn works_without_upsampling_and_on_odd_sizes() {
        let img = blob_image();
        let plain = detect(&img, &Detector::LARGE_SENSOR, ANYWHERE);
        assert!(plain.iter().all(|k| k.x >= 0.0 && k.x < 80.0 && k.y >= 0.0 && k.y < 80.0));
        let mut wide = Plane::new(97, 33);
        for (i, v) in wide.data.iter_mut().enumerate() {
            *v = ((i * 37 % 101) as f32) / 101.0;
        }
        for k in detect(&wide, &D, ANYWHERE) {
            assert!(k.x.is_finite() && k.y.is_finite() && k.size > 0.0 && (0.0..=360.0).contains(&k.angle));
        }
    }

    #[test]
    fn a_masked_area_does_not_use_up_the_keypoint_limit() {
        let all = detect(&blob_image(), &D, ANYWHERE);
        let right_half = |x: f32, _: f32| x >= 40.0;
        let visible: Vec<Keypoint> = all.iter().filter(|k| right_half(k.x, k.y)).cloned().collect();
        assert!(!visible.is_empty() && visible.len() < all.len());
        assert_eq!(detect(&blob_image(), &D, right_half), visible);
        let strongest = visible.iter().fold(&visible[0], |best, k| if k.response > best.response { k } else { best });
        assert_eq!(detect(&blob_image(), &Detector { max_keypoints: 1, ..D }, right_half), vec![strongest.clone()]);
    }

    #[test]
    fn flat_image_has_no_keypoints() {
        assert!(detect(&Plane::new(80, 80), &D, ANYWHERE).is_empty());
    }
}
