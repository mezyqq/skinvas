// Skin image + Minecraft player model geometry (units = skin pixels, y up, model faces +z).
use image::{Rgba, RgbaImage};

pub const HEAD: usize = 0;
pub const BODY: usize = 1;
pub const RARM: usize = 2;
pub const LARM: usize = 3;
pub const RLEG: usize = 4;
pub const LLEG: usize = 5;

#[derive(Clone, Copy)]
pub struct PartDef {
    pub min: [f32; 3],
    pub size: [u32; 3],
    pub uv: [u32; 2],
    pub ouv: [u32; 2],
    pub inflate: f32,
    pub pivot: [f32; 3],
}

pub fn parts(slim: bool) -> [PartDef; 6] {
    let aw = if slim { 3 } else { 4 };
    let a = aw as f32;
    let p = |min, size, uv, ouv, inflate, pivot| PartDef { min, size, uv, ouv, inflate, pivot };
    [
        p([-4., 24., -4.], [8, 8, 8], [0, 0], [32, 0], 0.5, [0., 24., 0.]),
        p([-4., 12., -2.], [8, 12, 4], [16, 16], [16, 32], 0.25, [0., 24., 0.]),
        p([-4. - a, 12., -2.], [aw, 12, 4], [40, 16], [40, 32], 0.25, [-5., 22., 0.]),
        p([4., 12., -2.], [aw, 12, 4], [32, 48], [48, 48], 0.25, [5., 22., 0.]),
        p([-4., 0., -2.], [4, 12, 4], [0, 16], [0, 32], 0.25, [-2., 12., 0.]),
        p([0., 0., -2.], [4, 12, 4], [16, 48], [0, 48], 0.25, [2., 12., 0.]),
    ]
}

/// One box face: texel rect [x, y, w, h] and the 3D mapping P(s,t) = o + s*du + t*dv, s,t in 0..1.
#[derive(Clone, Copy)]
pub struct Face {
    pub rect: [u32; 4],
    pub o: [f32; 3],
    pub du: [f32; 3],
    pub dv: [f32; 3],
    pub n: [f32; 3],
}

/// Faces in order: right(-x), front, left(+x), back, top, bottom.
pub fn box_faces(min: [f32; 3], max: [f32; 3], uv: [u32; 2], size: [u32; 3]) -> [Face; 6] {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let (w_, h_, d_) = (x1 - x0, y1 - y0, z1 - z0);
    let [u, v] = uv;
    let [w, h, d] = size;
    let f = |rect, o, du, dv, n| Face { rect, o, du, dv, n };
    [
        f([u, v + d, d, h], [x0, y1, z0], [0., 0., d_], [0., -h_, 0.], [-1., 0., 0.]),
        f([u + d, v + d, w, h], [x0, y1, z1], [w_, 0., 0.], [0., -h_, 0.], [0., 0., 1.]),
        f([u + d + w, v + d, d, h], [x1, y1, z1], [0., 0., -d_], [0., -h_, 0.], [1., 0., 0.]),
        f([u + 2 * d + w, v + d, w, h], [x1, y1, z0], [-w_, 0., 0.], [0., -h_, 0.], [0., 0., -1.]),
        f([u + d, v, w, d], [x0, y1, z0], [w_, 0., 0.], [0., 0., d_], [0., 1., 0.]),
        f([u + d + w, v, w, d], [x0, y0, z1], [w_, 0., 0.], [0., 0., -d_], [0., -1., 0.]),
    ]
}

pub fn part_faces(p: &PartDef, overlay: bool) -> [Face; 6] {
    let inf = if overlay { p.inflate } else { 0. };
    let min = [p.min[0] - inf, p.min[1] - inf, p.min[2] - inf];
    let max = [
        p.min[0] + p.size[0] as f32 + inf,
        p.min[1] + p.size[1] as f32 + inf,
        p.min[2] + p.size[2] as f32 + inf,
    ];
    box_faces(min, max, if overlay { p.ouv } else { p.uv }, p.size)
}

/// Which face (part, layer, rect) owns texel (x, y).
pub fn face_at(slim: bool, x: u32, y: u32) -> Option<(usize, bool, [u32; 4])> {
    let (i, overlay, k, _, _) = face_info(slim, x, y)?;
    Some((i, overlay, part_faces(&parts(slim)[i], overlay)[k].rect))
}

/// (part, overlay, face index as in `box_faces`, x and y inside the face).
pub fn face_info(slim: bool, x: u32, y: u32) -> Option<(usize, bool, usize, u32, u32)> {
    for (i, p) in parts(slim).iter().enumerate() {
        for overlay in [false, true] {
            for (k, f) in part_faces(p, overlay).iter().enumerate() {
                let [rx, ry, rw, rh] = f.rect;
                if x >= rx && x < rx + rw && y >= ry && y < ry + rh {
                    return Some((i, overlay, k, x - rx, y - ry));
                }
            }
        }
    }
    None
}

/// Base texel -> matching overlay texel of the same part.
pub fn to_overlay(slim: bool, x: u32, y: u32) -> (u32, u32) {
    match face_at(slim, x, y) {
        Some((i, false, _)) => {
            let p = parts(slim)[i];
            ((x + p.ouv[0]).wrapping_sub(p.uv[0]), (y + p.ouv[1]).wrapping_sub(p.uv[1]))
        }
        _ => (x, y),
    }
}

/// Load any PNG skin; legacy 64x32 skins are converted to 64x64.
pub fn load(bytes: &[u8]) -> Result<RgbaImage, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
    match img.dimensions() {
        (64, 64) => Ok(img),
        (64, 32) => Ok(upgrade_legacy(&img)),
        (w, h) => Err(format!("{w}x{h}")),
    }
}

pub fn detect_slim(img: &RgbaImage) -> bool {
    // Classic arms use column 54..56 of the right arm back face; slim leaves it empty.
    (20..32).all(|y| img.get_pixel(55, y)[3] == 0)
}

fn upgrade_legacy(old: &RgbaImage) -> RgbaImage {
    let mut img = RgbaImage::new(64, 64);
    image::imageops::replace(&mut img, old, 0, 0);
    // Legacy left limbs are mirrored copies of the right limbs.
    mirror_box(&mut img, [0, 16], [16, 48], [4, 12, 4]);
    mirror_box(&mut img, [40, 16], [32, 48], [4, 12, 4]);
    img
}

/// Copy a limb's texture to another limb, mirrored (left <-> right).
pub fn mirror_box(img: &mut RgbaImage, src: [u32; 2], dst: [u32; 2], size: [u32; 3]) {
    let z = [0.; 3];
    let s = box_faces(z, z, src, size);
    let d = box_faces(z, z, dst, size);
    // right<->left swap, front/back/top/bottom stay; every face flips horizontally.
    for (si, di) in [(0, 2), (1, 1), (2, 0), (3, 3), (4, 4), (5, 5)] {
        let ([sx, sy, w, h], [dx, dy, _, _]) = (s[si].rect, d[di].rect);
        let tmp: Vec<Rgba<u8>> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| *img.get_pixel(sx + x, sy + y)).collect();
        for y in 0..h {
            for x in 0..w {
                img.put_pixel(dx + w - 1 - x, dy + y, tmp[(y * w + x) as usize]);
            }
        }
    }
}

pub fn fill_rect(img: &mut RgbaImage, [x, y, w, h]: [u32; 4], c: [u8; 4]) {
    for yy in y..y + h {
        for xx in x..x + w {
            img.put_pixel(xx, yy, Rgba(c));
        }
    }
}

/// Mojang's default Steve texture; downloaded once and cached (it isn't ours to ship).
pub const STEVE_URL: &str = "https://textures.minecraft.net/texture/1a4af718455d4aab528e7a61f86fa25e6a369d1768dcb13f7df319a713eb810b";

pub fn steve_path() -> std::path::PathBuf {
    crate::library::dir().join("steve.png")
}

/// Skin for new documents: Steve if cached, otherwise our own starter skin.
pub fn starter() -> RgbaImage {
    if cfg!(test) {
        return default_skin();
    }
    std::fs::read(steve_path()).ok().and_then(|b| load(&b).ok()).unwrap_or_else(default_skin)
}

/// A plain starter skin (not Mojang's Steve): skin-tone head, shirt, pants, shoes.
pub fn default_skin() -> RgbaImage {
    let mut img = RgbaImage::new(64, 64);
    let skin = [214, 160, 120, 255];
    let hair = [70, 45, 25, 255];
    let shirt = [60, 140, 200, 255];
    let pants = [50, 55, 110, 255];
    let shoe = [60, 60, 60, 255];
    for p in parts(false) {
        for f in part_faces(&p, false) {
            fill_rect(&mut img, f.rect, skin);
        }
    }
    for i in [BODY, RARM, LARM] {
        for f in part_faces(&parts(false)[i], false) {
            let [x, y, w, h] = f.rect;
            let h2 = if i == BODY || f.rect[3] < 12 { h } else { 4 };
            fill_rect(&mut img, [x, y, w, h2], shirt);
        }
    }
    for i in [RLEG, LLEG] {
        for (k, f) in part_faces(&parts(false)[i], false).iter().enumerate() {
            let [x, y, w, h] = f.rect;
            fill_rect(&mut img, [x, y, w, h], pants);
            if k < 4 {
                fill_rect(&mut img, [x, y + h - 2, w, 2], shoe);
            } else if k == 5 {
                fill_rect(&mut img, f.rect, shoe);
            }
        }
    }
    let head = part_faces(&parts(false)[HEAD], false);
    fill_rect(&mut img, head[4].rect, hair);
    for k in [0, 2, 3] {
        let [x, y, w, _] = head[k].rect;
        fill_rect(&mut img, [x, y, w, if k == 3 { 6 } else { 3 }], hair);
    }
    let [fx, fy, _, _] = head[1].rect;
    fill_rect(&mut img, [fx, fy, 8, 2], hair);
    for (ex, c) in [(1, [255, 255, 255, 255]), (2, [40, 80, 160, 255]), (5, [40, 80, 160, 255]), (6, [255, 255, 255, 255])] {
        img.put_pixel(fx + ex, fy + 4, Rgba(c));
    }
    fill_rect(&mut img, [fx + 3, fy + 6, 2, 1], [150, 90, 70, 255]);
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_rects_match_minecraft_layout() {
        let f = part_faces(&parts(false)[HEAD], false);
        assert_eq!(f[1].rect, [8, 8, 8, 8]); // head front
        assert_eq!(f[3].rect, [24, 8, 8, 8]); // head back
        assert_eq!(f[4].rect, [8, 0, 8, 8]); // head top
        let o = part_faces(&parts(false)[LLEG], true);
        assert_eq!(o[1].rect, [4, 52, 4, 12]); // left leg overlay front
        assert_eq!(face_at(false, 20, 20).map(|f| f.0), Some(BODY));
        assert_eq!(to_overlay(false, 36, 52), (52, 52)); // left arm front -> its sleeve
        assert_eq!(to_overlay(false, 20, 52), (4, 52)); // left leg front -> its pants layer
    }

    #[test]
    fn legacy_upgrade_and_slim_detection() {
        let mut old = RgbaImage::new(64, 32);
        old.put_pixel(4, 20, Rgba([255, 0, 0, 255])); // right leg front, leftmost column
        old.put_pixel(55, 25, Rgba([1, 1, 1, 255]));
        let img = upgrade_legacy(&old);
        assert_eq!(img.dimensions(), (64, 64));
        assert_eq!(img.get_pixel(23, 52)[0], 255); // mirrored onto left leg front, rightmost column
        assert!(!detect_slim(&default_skin()));
    }
}
