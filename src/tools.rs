// Pure image operations behind the editor tools and dialogs.
use crate::colorpick::{hsv_to_rgb, rgb_to_hsv};
use crate::skin::{self, face_at, part_faces, parts};
use image::{Rgba, RgbaImage};

/// Texels covered by a brush of `size` centred on (x, y).
pub fn brush(x: i32, y: i32, size: u32, round: bool) -> Vec<(i32, i32)> {
    let s = size.max(1) as i32;
    let (lo, hi) = (-(s - 1) / 2, s / 2);
    let r = s as f64 / 2.;
    let mut out = vec![];
    for dy in lo..=hi {
        for dx in lo..=hi {
            // Centre of the offset cell relative to the brush centre.
            let (cx, cy) = (dx as f64 - (lo + hi) as f64 / 2., dy as f64 - (lo + hi) as f64 / 2.);
            if !round || s <= 2 || cx * cx + cy * cy <= r * r {
                out.push((x + dx, y + dy));
            }
        }
    }
    out
}

pub fn replace_color(img: &mut RgbaImage, from: [u8; 4], to: [u8; 4]) -> usize {
    let mut n = 0;
    for p in img.pixels_mut() {
        if p.0 == from {
            p.0 = to;
            n += 1;
        }
    }
    n
}

/// Which texels belong to the selected parts (None = whole image).
fn in_parts(slim: bool, sel: Option<&[usize]>, x: u32, y: u32) -> bool {
    match sel {
        None => true,
        Some(ps) => face_at(slim, x, y).is_some_and(|f| ps.contains(&f.0)),
    }
}

/// Hue shift in degrees, saturation and value multipliers.
pub fn adjust(orig: &RgbaImage, slim: bool, sel: Option<&[usize]>, dh: f64, ms: f64, mv: f64) -> RgbaImage {
    let mut img = orig.clone();
    for (x, y, p) in img.enumerate_pixels_mut() {
        if p[3] == 0 || !in_parts(slim, sel, x, y) {
            continue;
        }
        let [h, s, v] = rgb_to_hsv(p[0] as f64 / 255., p[1] as f64 / 255., p[2] as f64 / 255.);
        let [r, g, b] = hsv_to_rgb(h + dh, (s * ms).clamp(0., 1.), (v * mv).clamp(0., 1.));
        p.0 = [(r * 255.).round() as u8, (g * 255.).round() as u8, (b * 255.).round() as u8, p[3]];
    }
    img
}

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// Reduce the opaque colours to at most `k` (k-means with farthest-point init, deterministic).
pub fn quantize(orig: &RgbaImage, k: usize) -> RgbaImage {
    let px: Vec<[f64; 3]> = orig.pixels().filter(|p| p[3] > 0).map(|p| [p[0] as f64, p[1] as f64, p[2] as f64]).collect();
    if px.is_empty() || k == 0 {
        return orig.clone();
    }
    let mut c = vec![px[0]];
    while c.len() < k {
        let far = px.iter().copied().max_by(|a, b| {
            let da = c.iter().map(|m| dist2(*a, *m)).fold(f64::MAX, f64::min);
            let db = c.iter().map(|m| dist2(*b, *m)).fold(f64::MAX, f64::min);
            da.total_cmp(&db)
        });
        match far {
            Some(f) if c.iter().all(|m| dist2(f, *m) > 0.) => c.push(f),
            _ => break,
        }
    }
    let nearest = |p: [f64; 3], c: &[[f64; 3]]| (0..c.len()).min_by(|&a, &b| dist2(p, c[a]).total_cmp(&dist2(p, c[b]))).unwrap();
    for _ in 0..12 {
        let mut sum = vec![[0f64; 4]; c.len()];
        for p in &px {
            let i = nearest(*p, &c);
            for j in 0..3 {
                sum[i][j] += p[j];
            }
            sum[i][3] += 1.;
        }
        for (m, s) in c.iter_mut().zip(&sum) {
            if s[3] > 0. {
                *m = [s[0] / s[3], s[1] / s[3], s[2] / s[3]];
            }
        }
    }
    let mut img = orig.clone();
    for p in img.pixels_mut() {
        if p[3] > 0 {
            let m = c[nearest([p[0] as f64, p[1] as f64, p[2] as f64], &c)];
            p.0 = [m[0].round() as u8, m[1].round() as u8, m[2].round() as u8, p[3]];
        }
    }
    img
}

pub fn count_colors(img: &RgbaImage) -> usize {
    let mut v: Vec<[u8; 4]> = img.pixels().filter(|p| p[3] > 0).map(|p| p.0).collect();
    v.sort();
    v.dedup();
    v.len()
}

/// Copy whole parts (both layers) from `src` into `dst`.
pub fn transfer(dst: &mut RgbaImage, src: &RgbaImage, slim: bool, sel: &[usize]) {
    for &i in sel {
        for ov in [false, true] {
            for f in part_faces(&parts(slim)[i], ov) {
                let [x, y, w, h] = f.rect;
                for yy in y..y + h {
                    for xx in x..x + w {
                        dst.put_pixel(xx, yy, *src.get_pixel(xx, yy));
                    }
                }
            }
        }
    }
}

pub fn crop(img: &RgbaImage, [x, y, w, h]: [u32; 4]) -> RgbaImage {
    image::imageops::crop_imm(img, x, y, w, h).to_image()
}

/// Stamp `piece` at (x, y); transparent texels of the piece are copied too (it's a move, not a blend).
pub fn stamp(img: &mut RgbaImage, piece: &RgbaImage, x: i32, y: i32) {
    for (px, py, p) in piece.enumerate_pixels() {
        let (tx, ty) = (x + px as i32, y + py as i32);
        if (0..64).contains(&tx) && (0..64).contains(&ty) {
            img.put_pixel(tx as u32, ty as u32, *p);
        }
    }
}

pub fn clear_rect(img: &mut RgbaImage, [x, y, w, h]: [u32; 4]) {
    skin::fill_rect(img, [x, y, w.min(64 - x), h.min(64 - y)], [0; 4]);
}

#[derive(Debug, PartialEq)]
pub enum Issue {
    /// Base-layer texels with transparency: Minecraft shows them black.
    TransparentBase(usize),
    /// Pixels outside every UV face: never visible, only bloat the file.
    Unused(usize),
    /// Half-transparent outer-layer texels: rendered inconsistently between versions.
    SemiTransparent(usize),
}

pub fn check(img: &RgbaImage, slim: bool) -> Vec<Issue> {
    let (mut tb, mut un, mut st) = (0, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        match face_at(slim, x, y) {
            Some((_, false, _)) if p[3] < 255 => tb += 1,
            Some((_, true, _)) if p[3] > 0 && p[3] < 255 => st += 1,
            None if p[3] > 0 => un += 1,
            _ => {}
        }
    }
    [(tb, Issue::TransparentBase(tb)), (un, Issue::Unused(un)), (st, Issue::SemiTransparent(st))].into_iter().filter(|(n, _)| *n > 0).map(|(_, i)| i).collect()
}

/// Fixes every issue `check` reports.
pub fn fix(img: &mut RgbaImage, slim: bool) {
    let mut out = img.clone();
    for (x, y, p) in img.enumerate_pixels() {
        match face_at(slim, x, y) {
            Some((_, false, rect)) if p[3] < 255 => {
                // Fill with the face's average opaque colour, or keep the RGB and make it opaque.
                let [rx, ry, rw, rh] = rect;
                let (mut s, mut n) = ([0u32; 3], 0);
                for yy in ry..ry + rh {
                    for xx in rx..rx + rw {
                        let q = img.get_pixel(xx, yy);
                        if q[3] == 255 {
                            (0..3).for_each(|i| s[i] += q[i] as u32);
                            n += 1;
                        }
                    }
                }
                let c = if n > 0 { [(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8, 255] } else { [p[0], p[1], p[2], 255] };
                out.put_pixel(x, y, Rgba(c));
            }
            Some((_, true, _)) if p[3] > 0 && p[3] < 255 => out.put_pixel(x, y, Rgba([p[0], p[1], p[2], if p[3] >= 128 { 255 } else { 0 }])),
            None if p[3] > 0 => out.put_pixel(x, y, Rgba([0; 4])),
            _ => {}
        }
    }
    *img = out;
}

/// Small xorshift so random skins need no extra dependency.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn f(&mut self) -> f64 {
        (self.next() % 10_000) as f64 / 10_000.
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[(self.next() % v.len() as u64) as usize]
    }
}

fn hsv(h: f64, s: f64, v: f64) -> [u8; 4] {
    let [r, g, b] = hsv_to_rgb(h, s, v);
    [(r * 255.) as u8, (g * 255.) as u8, (b * 255.) as u8, 255]
}

/// A random but coherent character: skin tone, hair, shirt, pants, shoes, light texture noise.
pub fn random_skin(seed: u64) -> RgbaImage {
    let mut r = Rng::new(seed);
    let tones = [[255, 220, 177], [241, 194, 150], [224, 172, 125], [198, 134, 96], [141, 85, 54], [94, 58, 38]];
    let t = r.pick(&tones);
    let tone = [t[0], t[1], t[2], 255];
    let hair = if r.f() < 0.7 { hsv(20. + r.f() * 25., 0.5 + r.f() * 0.4, 0.15 + r.f() * 0.6) } else { hsv(r.f() * 360., 0.6, 0.8) };
    let shirt = hsv(r.f() * 360., 0.35 + r.f() * 0.5, 0.35 + r.f() * 0.6);
    let pants = hsv(r.f() * 360., 0.2 + r.f() * 0.5, 0.2 + r.f() * 0.5);
    let shoes = hsv(r.f() * 360., r.f() * 0.4, 0.1 + r.f() * 0.3);
    let eyes = hsv(r.f() * 360., 0.6, 0.4 + r.f() * 0.4);
    let long_sleeves = r.f() < 0.5;
    let hair_len = 1 + (r.next() % 4) as u32;
    let mut img = RgbaImage::new(64, 64);
    let ps = parts(false);
    for (i, p) in ps.iter().enumerate() {
        for (k, f) in part_faces(p, false).iter().enumerate() {
            let [x, y, w, h] = f.rect;
            let base = match i {
                skin::HEAD => tone,
                skin::BODY => shirt,
                skin::RARM | skin::LARM => tone,
                _ => pants,
            };
            skin::fill_rect(&mut img, f.rect, base);
            if matches!(i, skin::RARM | skin::LARM) && k < 5 {
                let sleeve = if long_sleeves { h.saturating_sub(3) } else { 4.min(h) };
                skin::fill_rect(&mut img, [x, y, w, if k == 4 { h } else { sleeve }], shirt);
            }
            if matches!(i, skin::RLEG | skin::LLEG) {
                if k < 4 {
                    skin::fill_rect(&mut img, [x, y + h - 2, w, 2], shoes);
                } else if k == 5 {
                    skin::fill_rect(&mut img, f.rect, shoes);
                }
            }
        }
    }
    let head = part_faces(&ps[skin::HEAD], false);
    skin::fill_rect(&mut img, head[4].rect, hair);
    for k in [0, 2, 3] {
        let [x, y, w, _] = head[k].rect;
        skin::fill_rect(&mut img, [x, y, w, if k == 3 { 4 + hair_len } else { 1 + hair_len }], hair);
    }
    let [fx, fy, _, _] = head[1].rect;
    skin::fill_rect(&mut img, [fx, fy, 8, hair_len.min(3)], hair);
    for (ex, c) in [(1, [255, 255, 255, 255]), (2, eyes), (5, eyes), (6, [255, 255, 255, 255])] {
        img.put_pixel(fx + ex, fy + 4, Rgba(c));
    }
    let mouth = [(t[0] as f64 * 0.7) as u8, (t[1] as f64 * 0.55) as u8, (t[2] as f64 * 0.55) as u8, 255];
    skin::fill_rect(&mut img, [fx + 3, fy + 6, 2, 1], mouth);
    // Gentle per-texel noise so it doesn't look flat.
    for p in img.pixels_mut() {
        if p[3] > 0 {
            let n = (r.f() - 0.5) * 18.;
            for c in 0..3 {
                p[c] = (p[c] as f64 + n).clamp(0., 255.) as u8;
            }
        }
    }
    img
}

pub fn rotate90(img: &RgbaImage) -> RgbaImage {
    image::imageops::rotate90(img)
}

pub fn flip_h(img: &RgbaImage) -> RgbaImage {
    image::imageops::flip_horizontal(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brush_shapes() {
        assert_eq!(brush(5, 5, 1, true), vec![(5, 5)]);
        assert_eq!(brush(5, 5, 3, false).len(), 9);
        let round = brush(5, 5, 5, true);
        assert!(round.len() < 25 && round.contains(&(5, 5)) && !round.contains(&(3, 3)));
    }

    #[test]
    fn quantize_limits_colours() {
        let img = random_skin(42);
        assert!(count_colors(&img) > 8);
        assert!(count_colors(&quantize(&img, 8)) <= 8);
    }

    #[test]
    fn check_and_fix() {
        let mut img = skin::default_skin();
        assert!(check(&img, false).is_empty(), "{:?}", check(&img, false));
        img.put_pixel(0, 0, Rgba([1, 2, 3, 255])); // unused corner
        img.put_pixel(9, 9, Rgba([1, 2, 3, 0])); // hole in the face
        assert_eq!(check(&img, false), vec![Issue::TransparentBase(1), Issue::Unused(1)]);
        fix(&mut img, false);
        assert!(check(&img, false).is_empty());
    }

    #[test]
    fn props_have_pixels() {
        let sk = skin::default_skin();
        for k in 1..ITEMS.len() {
            assert!(item_sprite(k, &sk, false).pixels().any(|p| p[3] > 0), "item {k}");
        }
        let a = armor_texture(0x4EE6D6, false);
        assert!(a.get_pixel(12, 2)[3] > 0); // helmet top
        assert_eq!(a.get_pixel(12, 13)[3], 0); // face opening
    }

    #[test]
    fn transfer_and_replace() {
        let mut a = RgbaImage::new(64, 64);
        let b = skin::default_skin();
        transfer(&mut a, &b, false, &[skin::HEAD]);
        assert_eq!(a.get_pixel(12, 12), b.get_pixel(12, 12));
        assert_eq!(a.get_pixel(20, 20)[3], 0);
        let c = b.get_pixel(20, 20).0;
        let mut d = b.clone();
        assert!(replace_color(&mut d, c, [1, 1, 1, 255]) > 10);
    }
}

// --- 3D props: held items and armor, drawn procedurally (no Mojang textures shipped) ---

pub const ITEMS: [&str; 5] = ["item_none", "item_sword", "item_pickaxe", "item_shield", "item_totem"];
pub const ARMORS: [(&str, u32); 6] = [("armor_none", 0), ("armor_leather", 0xA06540), ("armor_iron", 0xD8D8D8), ("armor_gold", 0xF6D23F), ("armor_diamond", 0x4EE6D6), ("armor_netherite", 0x4A4245)];

fn put(img: &mut RgbaImage, pts: &[(u32, u32)], c: [u8; 4]) {
    for &(x, y) in pts {
        if x < img.width() && y < img.height() {
            img.put_pixel(x, y, Rgba(c));
        }
    }
}

fn shade(c: [u8; 4], k: f64) -> [u8; 4] {
    [(c[0] as f64 * k).min(255.) as u8, (c[1] as f64 * k).min(255.) as u8, (c[2] as f64 * k).min(255.) as u8, c[3]]
}

/// 16x16 sprite of a held item; handle at bottom-left, tip at top-right.
pub fn item_sprite(kind: usize, skin_img: &RgbaImage, slim: bool) -> RgbaImage {
    let mut img = RgbaImage::new(16, 16);
    let (wood, dark_wood) = ([137, 98, 52, 255], [86, 60, 30, 255]);
    let (steel, steel_hi, steel_lo) = ([200, 205, 210, 255], [240, 244, 248, 255], [110, 115, 125, 255]);
    match kind {
        1 => {
            for i in 0..10 {
                put(&mut img, &[(5 + i, 10 - i)], steel_hi);
                put(&mut img, &[(6 + i, 10 - i), (5 + i, 11 - i)], steel);
                put(&mut img, &[(6 + i, 11 - i)], steel_lo);
            }
            put(&mut img, &[(2, 9), (3, 10), (4, 11), (5, 12), (6, 13)], [70, 70, 80, 255]);
            put(&mut img, &[(2, 13), (3, 12), (4, 12), (3, 13)], wood);
            put(&mut img, &[(1, 14), (2, 14), (1, 15)], dark_wood);
        }
        2 => {
            for i in 0..12 {
                put(&mut img, &[(1 + i, 14 - i)], if i % 2 == 0 { wood } else { dark_wood });
            }
            let head = [(5, 1), (6, 1), (7, 1), (8, 1), (9, 2), (10, 2), (11, 3), (12, 4), (13, 5), (13, 6), (14, 7), (14, 8), (14, 9), (14, 10)];
            put(&mut img, &head, steel);
            put(&mut img, &head.map(|(x, y)| (x - 1, y + 1)), steel_lo);
            put(&mut img, &[(4, 1), (14, 11)], steel_hi);
        }
        3 => {
            for y in 1..15 {
                for x in 3..13 {
                    let edge = x == 3 || x == 12 || y == 1 || y == 14;
                    put(&mut img, &[(x, y)], if edge { steel_lo } else if (x + y) % 3 == 0 { dark_wood } else { wood });
                }
            }
            put(&mut img, &[(7, 7), (8, 7), (7, 8), (8, 8)], steel_hi);
        }
        4 => img = crate::packs::totem(skin_img, slim),
        _ => {}
    }
    img
}

/// Armor texture in the skin layout (helmet on the head, chestplate on body+arms, leggings and boots on legs).
pub fn armor_texture(rgb: u32, slim: bool) -> RgbaImage {
    let base = [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255];
    let mut img = RgbaImage::new(64, 64);
    let mut r = Rng::new(rgb as u64 + 7);
    for (i, p) in parts(slim).iter().enumerate() {
        for (k, f) in part_faces(p, false).iter().enumerate() {
            let [x, y, w, h] = f.rect;
            for yy in 0..h {
                for xx in 0..w {
                    let edge = xx == 0 || yy == 0 || xx + 1 == w || yy + 1 == h;
                    let mut c = shade(base, if edge { 0.72 } else { 0.92 + r.f() * 0.16 });
                    let keep = match i {
                        // Helmet: open face and bottom.
                        skin::HEAD => k != 5 && !(k == 1 && yy >= 3 && xx >= 1 && xx + 1 < w),
                        skin::BODY => k != 5,
                        // Shoulder pads only.
                        skin::RARM | skin::LARM => k == 4 || yy < 5,
                        _ => {
                            if k < 4 && yy + 4 >= h {
                                c = shade(c, 0.8); // boots
                            }
                            k != 4
                        }
                    };
                    if keep {
                        img.put_pixel(x + xx, y + yy, Rgba(c));
                    }
                }
            }
        }
    }
    img
}

// --- palettes, picture import, PBR, 3D-print and Bedrock export ---

pub const HARMONIES: [&str; 7] = ["harm_complement", "harm_analog", "harm_triad", "harm_split", "harm_square", "harm_shades", "harm_pastel"];

/// Five colours that go with `base` for the given scheme (index into HARMONIES).
pub fn harmony(base: [u8; 4], scheme: usize) -> Vec<[u8; 4]> {
    let [h, s, v] = rgb_to_hsv(base[0] as f64 / 255., base[1] as f64 / 255., base[2] as f64 / 255.);
    let (s, v) = (s.max(0.25), v.max(0.25));
    let hsv = |h: f64, s: f64, v: f64| {
        let [r, g, b] = hsv_to_rgb(h, s.clamp(0., 1.), v.clamp(0., 1.));
        [(r * 255.).round() as u8, (g * 255.).round() as u8, (b * 255.).round() as u8, 255]
    };
    let hues: Vec<(f64, f64, f64)> = match scheme {
        0 => vec![(0., s, v), (0., s, v * 0.7), (180., s, v), (180., s * 0.6, v), (180., s, v * 0.7)],
        1 => vec![(-60., s, v), (-30., s, v), (0., s, v), (30., s, v), (60., s, v)],
        2 => vec![(0., s, v), (120., s, v), (240., s, v), (120., s * 0.6, v), (240., s * 0.6, v)],
        3 => vec![(0., s, v), (150., s, v), (210., s, v), (150., s, v * 0.7), (210., s, v * 0.7)],
        4 => vec![(0., s, v), (90., s, v), (180., s, v), (270., s, v), (0., s * 0.5, v)],
        5 => (0..5).map(|i| (0., s, 0.2 + i as f64 * 0.2)).collect(),
        _ => (0..5).map(|i| (i as f64 * 72., 0.3, 0.95)).collect(),
    };
    hues.into_iter().map(|(dh, s, v)| hsv(h + dh, s, v)).collect()
}

pub const PIC_TARGETS: [&str; 4] = ["pic_front", "pic_face", "pic_chest", "pic_back"];

/// Paints a picture onto the skin (base layer): whole front, face, chest or back.
pub fn picture(dst: &mut RgbaImage, pic: &image::DynamicImage, slim: bool, target: usize) {
    use image::imageops::FilterType;
    let ps = parts(slim);
    let put = |dst: &mut RgbaImage, rect: [u32; 4], src: &RgbaImage, sx: u32, sy: u32| {
        for y in 0..rect[3] {
            for x in 0..rect[2] {
                let c = src.get_pixel(sx + x, sy + y);
                if c[3] > 0 {
                    dst.put_pixel(rect[0] + x, rect[1] + y, Rgba([c[0], c[1], c[2], 255]));
                }
            }
        }
    };
    match target {
        0 => {
            // Same 16x32 front view as `packs::front`, written back into each part's front face.
            let f = pic.resize_exact(16, 32, FilterType::Triangle).to_rgba8();
            for p in &ps {
                let r = part_faces(p, false)[1].rect;
                let (ox, oy) = ((p.min[0] + 8.) as u32, (32. - p.min[1] - p.size[1] as f32) as u32);
                put(dst, r, &f, ox, oy);
            }
        }
        _ => {
            let (part, face) = [(skin::HEAD, 1), (skin::HEAD, 1), (skin::BODY, 1), (skin::BODY, 3)][target];
            let r = part_faces(&ps[part], false)[face].rect;
            let f = pic.resize_exact(r[2], r[3], FilterType::Triangle).to_rgba8();
            put(dst, r, &f, 0, 0);
        }
    }
}

fn luma(p: &Rgba<u8>) -> f64 {
    (0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64) / 255.
}

/// LabPBR normal map (_n): brightness treated as height, Sobel per UV face so faces don't bleed.
pub fn normal_map(img: &RgbaImage, slim: bool) -> RgbaImage {
    let mut out = RgbaImage::new(64, 64);
    for (x, y, p) in img.enumerate_pixels() {
        let Some((_, _, [rx, ry, rw, rh])) = face_at(slim, x, y) else { continue };
        if p[3] == 0 {
            continue;
        }
        let h = |dx: i32, dy: i32| {
            let (sx, sy) = ((x as i32 + dx).clamp(rx as i32, (rx + rw - 1) as i32), (y as i32 + dy).clamp(ry as i32, (ry + rh - 1) as i32));
            luma(img.get_pixel(sx as u32, sy as u32))
        };
        let gx = h(1, -1) + 2. * h(1, 0) + h(1, 1) - h(-1, -1) - 2. * h(-1, 0) - h(-1, 1);
        let gy = h(-1, 1) + 2. * h(0, 1) + h(1, 1) - h(-1, -1) - 2. * h(0, -1) - h(1, -1);
        let (nx, ny) = (-gx * 1.5, -gy * 1.5);
        let l = (nx * nx + ny * ny + 1.).sqrt();
        let enc = |v: f64| ((v / l * 0.5 + 0.5) * 255.).round() as u8;
        // R,G = normal; B = ambient occlusion (255 = none); A = height.
        out.put_pixel(x, y, Rgba([enc(nx), enc(ny), 255, (luma(p) * 255.) as u8]));
    }
    out
}

/// LabPBR specular map (_s): smoothness from brightness, dielectric, no emission.
pub fn specular_map(img: &RgbaImage) -> RgbaImage {
    RgbaImage::from_fn(64, 64, |x, y| {
        let p = img.get_pixel(x, y);
        if p[3] == 0 { Rgba([0; 4]) } else { Rgba([(40. + luma(p) * 120.) as u8, 10, 0, 255]) }
    })
}

/// Binary STL of the model (base + voxel outer layer). `height_mm` = height of the figure.
pub fn stl(img: &RgbaImage, slim: bool, height_mm: f32) -> Vec<u8> {
    let q = crate::render::build(img, &crate::render::ModelOpts { slim, base: true, overlay: true, voxels: true, thickness: 1. });
    let k = height_mm / 32.;
    let mut out = vec![0u8; 80];
    out[..20].copy_from_slice(b"Skinvas figure STL  ");
    out.extend(((q.len() * 2) as u32).to_le_bytes());
    for quad in &q {
        // Wind each triangle so its normal matches the face normal.
        let p = quad.p.map(|v| [v[0] * k, v[2] * k, v[1] * k]);
        let n = [quad.n[0], quad.n[2], quad.n[1]];
        for tri in [[0, 1, 2], [0, 2, 3]] {
            let [a, b, c] = tri.map(|i| p[i]);
            let cr = [(b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1]), (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]), (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])];
            let (b, c) = if cr[0] * n[0] + cr[1] * n[1] + cr[2] * n[2] < 0. { (c, b) } else { (b, c) };
            for v in [n, a, b, c] {
                for f in v {
                    out.extend(f.to_le_bytes());
                }
            }
            out.extend([0u8, 0]);
        }
    }
    out
}

fn uuid(r: &mut Rng) -> String {
    let a = r.next();
    let b = r.next();
    // Version 4 / RFC 4122 variant bits.
    let b = (b & 0x3fff_ffff_ffff_ffff) | 0x8000_0000_0000_0000;
    let a = (a & 0xffff_ffff_ffff_0fff) | 0x0000_0000_0000_4000;
    format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}", a >> 32, (a >> 16) & 0xffff, a & 0xffff, b >> 48, b & 0xffff_ffff_ffff)
}

/// Bedrock skin pack (.mcpack): manifest + skins.json + texture + names.
pub fn mcpack(img: &RgbaImage, slim: bool, name: &str) -> Vec<u8> {
    use std::io::Write;
    let mut r = Rng::new(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64);
    let n = serde_json::to_string(name).unwrap();
    let manifest = format!(r#"{{"format_version":1,"header":{{"name":{n},"uuid":"{}","version":[1,0,0]}},"modules":[{{"type":"skin_pack","uuid":"{}","version":[1,0,0]}}]}}"#, uuid(&mut r), uuid(&mut r));
    let geo = if slim { "geometry.humanoid.customSlim" } else { "geometry.humanoid.custom" };
    let skins = format!(r#"{{"skins":[{{"localization_name":"skin1","geometry":"{geo}","texture":"skin.png","type":"free"}}],"serialize_name":"skinvas","localization_name":"skinvas"}}"#);
    let clean: String = name.chars().filter(|c| *c != '\n' && *c != '=').collect();
    let lang = format!("skinpack.skinvas={clean}\nskin.skinvas.skin1={clean}\n");
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (f, d) in [("manifest.json", manifest.into_bytes()), ("skins.json", skins.into_bytes()), ("skin.png", png.into_inner()), ("texts/en_US.lang", lang.into_bytes())] {
        z.start_file(f, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(&d).unwrap();
    }
    z.finish().unwrap().into_inner()
}

/// Folders where Minecraft and launchers cache every skin they have downloaded.
pub fn skin_caches() -> Vec<std::path::PathBuf> {
    let h = gtk::glib::home_dir();
    [".minecraft/assets/skins", ".local/share/PrismLauncher/assets/skins", ".var/app/org.prismlauncher.PrismLauncher/data/PrismLauncher/assets/skins", ".local/share/multimc/assets/skins", ".local/share/ModrinthApp/meta/assets/skins", ".tlauncher/legacy/Minecraft/game/assets/skins"]
        .iter()
        .map(|p| h.join(p))
        .filter(|p| p.is_dir())
        .collect()
}

/// 64x64 skins in the caches (64x32 files there are capes, so they're skipped), newest first.
pub fn cached_skins() -> Vec<std::path::PathBuf> {
    fn walk(d: &std::path::Path, out: &mut Vec<(std::time::SystemTime, std::path::PathBuf)>, depth: u32) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && depth < 3 {
                walk(&p, out, depth + 1);
            } else if image::image_dimensions(&p).is_ok_and(|d| d == (64, 64)) {
                out.push((e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p));
            }
        }
    }
    let mut v = vec![];
    for d in skin_caches() {
        walk(&d, &mut v, 0);
    }
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests2 {
    use super::*;

    /// Dev helper: OUT=file.stl cargo test stl_dump -- --ignored
    #[test]
    #[ignore]
    fn stl_dump() {
        std::fs::write(std::env::var("OUT").unwrap(), stl(&crate::tour::demo_skin(3), false, 80.)).unwrap();
    }

    #[test]
    fn harmonies_and_maps() {
        for s in 0..HARMONIES.len() {
            assert_eq!(harmony([200, 40, 40, 255], s).len(), 5);
        }
        let comp = harmony([255, 0, 0, 255], 0);
        assert!(comp[2][1] > 200 && comp[2][2] > 200 && comp[2][0] < 60, "{:?}", comp[2]); // cyan opposite red
        let sk = skin::default_skin();
        let n = normal_map(&sk, false);
        assert_eq!(n.get_pixel(0, 0)[3], 0); // unused area stays empty
        assert!(n.get_pixel(12, 12)[2] == 255);
        let stl = stl(&sk, false, 64.);
        let tris = u32::from_le_bytes(stl[80..84].try_into().unwrap()) as usize;
        assert_eq!(stl.len(), 84 + tris * 50);
        let z = zip::ZipArchive::new(std::io::Cursor::new(mcpack(&sk, true, "Cat"))).unwrap();
        let names: Vec<String> = z.file_names().flatten().map(String::from).collect();
        assert!(names.contains(&"manifest.json".to_string()) && names.contains(&"skin.png".to_string()));
        let mut img = RgbaImage::new(64, 64);
        picture(&mut img, &image::DynamicImage::ImageRgba8(RgbaImage::from_pixel(10, 10, Rgba([9, 9, 9, 255]))), false, 1);
        assert_eq!(img.get_pixel(12, 12).0, [9, 9, 9, 255]);
        assert_eq!(img.get_pixel(20, 20)[3], 0);
    }
}
