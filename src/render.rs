// Software rasterizer for the player model. Besides colour it writes a pick buffer
// (which skin texel is under each pixel), which is what makes painting on the 3D model work.
use crate::skin::{self, Face};
use image::RgbaImage;

type V3 = [f32; 3];

#[derive(Clone, Copy)]
pub struct Quad {
    pub p: [V3; 4],
    pub uv: [[f32; 2]; 4],
    pub rect: [u32; 4],
    pub n: V3,
    pub part: u8,
    pub overlay: bool,
    pub cull: bool,
}

pub struct ModelOpts {
    pub slim: bool,
    pub base: bool,
    pub overlay: bool,
    pub voxels: bool,
    pub thickness: f32,
}

/// Per-part rotation (radians, x/y/z) applied around the part pivot.
pub type Pose = [V3; 6];

pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub fov: f32,
    pub target: V3,
}

pub struct Frame {
    pub w: usize,
    /// Cairo ARGB32 (premultiplied BGRA on little endian).
    pub color: Vec<u8>,
    pub pick: Vec<u32>,
}

impl Frame {
    /// Texel and layer under pixel (x, y).
    pub fn pick_at(&self, x: usize, y: usize) -> Option<(u32, u32, bool)> {
        let v = *self.pick.get(y * self.w + x)?;
        (v != 0).then(|| (((v - 1) & 63), ((v - 1) >> 6) & 63, (v - 1) >> 12 == 1))
    }
}

fn add(a: V3, b: V3) -> V3 { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
fn mul(a: V3, s: f32) -> V3 { [a[0] * s, a[1] * s, a[2] * s] }
fn dot(a: V3, b: V3) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn norm(a: V3) -> V3 { mul(a, 1. / dot(a, a).sqrt().max(1e-9)) }

fn face_quad(f: &Face, part: usize, overlay: bool, cull: bool) -> Quad {
    let [rx, ry, rw, rh] = f.rect;
    let (x0, y0, x1, y1) = (rx as f32, ry as f32, (rx + rw) as f32, (ry + rh) as f32);
    Quad {
        p: [f.o, add(f.o, f.du), add(add(f.o, f.du), f.dv), add(f.o, f.dv)],
        uv: [[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
        rect: f.rect,
        n: f.n,
        part: part as u8,
        overlay,
        cull,
    }
}

/// Builds the mesh. In voxel mode ("3D skin layers") each opaque overlay texel becomes a small cube.
pub fn build(img: &RgbaImage, o: &ModelOpts) -> Vec<Quad> {
    let mut out = Vec::new();
    let opaque = |x: u32, y: u32| img.get_pixel(x, y)[3] > 0;
    for (i, p) in skin::parts(o.slim).iter().enumerate() {
        if o.base {
            out.extend(skin::part_faces(p, false).iter().map(|f| face_quad(f, i, false, true)));
        }
        if !o.overlay {
            continue;
        }
        if !o.voxels {
            out.extend(skin::part_faces(p, true).iter().map(|f| face_quad(f, i, true, false)));
            continue;
        }
        // Voxels sit on the base box surface and grow outward.
        for f in skin::part_faces(p, false) {
            let [rx, ry, rw, rh] = f.rect;
            let (ox, oy) = (p.ouv[0] as i64 - p.uv[0] as i64, p.ouv[1] as i64 - p.uv[1] as i64);
            let (e1, e2, e3) = (mul(f.du, 1. / rw as f32), mul(f.dv, 1. / rh as f32), mul(f.n, o.thickness));
            for j in 0..rh {
                for k in 0..rw {
                    let (tx, ty) = ((rx + k) as i64 + ox, (ry + j) as i64 + oy);
                    let (tx, ty) = (tx as u32, ty as u32);
                    if !opaque(tx, ty) {
                        continue;
                    }
                    let a = add(add(f.o, mul(e1, k as f32)), mul(e2, j as f32));
                    let rect = [tx, ty, 1, 1];
                    let mut q = |o: V3, du: V3, dv: V3, n: V3| {
                        out.push(face_quad(&Face { rect, o, du, dv, n }, i, true, true));
                    };
                    let n1 = norm(e1);
                    let n2 = norm(e2);
                    // Outer cap, then the four sides (skip sides hidden by a neighbouring voxel).
                    q(add(a, e3), e1, e2, f.n);
                    if k == 0 || !opaque(tx - 1, ty) { q(a, e3, e2, mul(n1, -1.)); }
                    if k + 1 == rw || !opaque(tx + 1, ty) { q(add(a, e1), e2, e3, n1); }
                    if j == 0 || !opaque(tx, ty - 1) { q(a, e1, e3, mul(n2, -1.)); }
                    if j + 1 == rh || !opaque(tx, ty + 1) { q(add(a, e2), e3, e1, n2); }
                }
            }
        }
    }
    out
}

fn rotate(v: V3, r: V3) -> V3 {
    let [mut x, mut y, mut z] = v;
    let (s, c) = r[0].sin_cos();
    (y, z) = (y * c - z * s, y * s + z * c);
    let (s, c) = r[1].sin_cos();
    (x, z) = (x * c + z * s, -x * s + z * c);
    let (s, c) = r[2].sin_cos();
    (x, y) = (x * c - y * s, x * s + y * c);
    [x, y, z]
}

pub fn walk_pose(t: f32, amount: f32) -> Pose {
    let a = t.sin() * amount;
    let mut p = [[0.; 3]; 6];
    p[skin::RARM][0] = a;
    p[skin::LARM][0] = -a;
    p[skin::RLEG][0] = -a;
    p[skin::LLEG][0] = a;
    p
}

#[derive(Clone, Copy, PartialEq)]
pub enum Light {
    /// Soft light from the camera side (editor default).
    Soft,
    Flat,
    /// Fixed per-direction brightness like Minecraft entities.
    Minecraft,
    /// Dark ambient + a warm torch from the front right.
    Night,
}

/// One mesh in the scene with its own texture, position and pose.
pub struct Model<'a> {
    pub quads: &'a [Quad],
    pub img: &'a RgbaImage,
    pub slim: bool,
    pub offset: V3,
    pub pose: Pose,
    /// 0 = assembled, 1 = parts pushed apart (exploded view).
    pub explode: f32,
    /// Writes the pick buffer (only the edited skin should).
    pub pick: bool,
}

/// Renders into a w*h frame. `bg` is straight RGBA.
pub fn render(quads: &[Quad], img: &RgbaImage, slim: bool, pose: &Pose, cam: &Camera, w: usize, h: usize, bg: [u8; 4], shading: bool) -> Frame {
    let m = Model { quads, img, slim, offset: [0.; 3], pose: *pose, explode: 0., pick: true };
    render_scene(&[m], cam, w, h, bg, bg, if shading { Light::Soft } else { Light::Flat })
}

pub fn render_scene(models: &[Model], cam: &Camera, w: usize, h: usize, bg_top: [u8; 4], bg_bottom: [u8; 4], light: Light) -> Frame {
    let n = w * h;
    let mut color = vec![0u8; n * 4];
    for y in 0..h {
        let k = y as f32 / h.max(2) as f32;
        let c: [f32; 4] = std::array::from_fn(|i| bg_top[i] as f32 * (1. - k) + bg_bottom[i] as f32 * k);
        let a = c[3] / 255.;
        let px = [(c[2] * a) as u8, (c[1] * a) as u8, (c[0] * a) as u8, c[3] as u8];
        for x in 0..w {
            color[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&px);
        }
    }
    let mut depth = vec![0f32; n];
    let mut pick = vec![0u32; n];
    let f = (w.min(h) as f32 / 2.) / (cam.fov.to_radians() / 2.).tan();
    let soft = norm([0.35, 0.7, 1.0]);
    let torch = norm([-0.8, 0.5, 0.9]);
    let view = |v: V3| {
        let v = rotate([v[0] - cam.target[0], v[1] - cam.target[1], v[2] - cam.target[2]], [0., cam.yaw, 0.]);
        let mut v = rotate(v, [cam.pitch, 0., 0.]);
        v[2] -= cam.dist;
        v
    };
    for m in models {
        let ps = skin::parts(m.slim);
        for q in m.quads {
            let (part, r) = (&ps[q.part as usize], m.pose[q.part as usize]);
            let piv = part.pivot;
            // Exploded view: push each part away from the body centre.
            let c = [part.min[0] + part.size[0] as f32 / 2., part.min[1] + part.size[1] as f32 / 2., part.min[2] + part.size[2] as f32 / 2.];
            let ex = add(mul([c[0], c[1] - 16., c[2]], m.explode * 0.45), m.offset);
            let world = |v: V3| add(add(rotate([v[0] - piv[0], v[1] - piv[1], v[2] - piv[2]], r), piv), ex);
            let vs = q.p.map(|v| view(world(v)));
            if vs.iter().any(|v| v[2] > -0.5) {
                continue;
            }
            let nw = rotate(q.n, r);
            let nv = rotate(rotate(nw, [0., cam.yaw, 0.]), [cam.pitch, 0., 0.]);
            if q.cull && dot(nv, mul(vs[0], -1.)) <= 0. {
                continue;
            }
            let (shade, tint) = match light {
                Light::Soft => (0.55 + 0.45 * dot(nv, soft).abs(), [1.; 3]),
                Light::Flat => (1., [1.; 3]),
                Light::Minecraft => {
                    let a = nw.map(f32::abs);
                    (if nw[1] > 0.5 { 1. } else if nw[1] < -0.5 { 0.5 } else if a[2] >= a[0] { 0.8 } else { 0.6 }, [1.; 3])
                }
                Light::Night => (0.2 + 0.95 * dot(nw, torch).max(0.), [1., 0.78, 0.5]),
            };
            let s = std::array::from_fn::<[f32; 5], 4, _>(|i| {
                let iz = 1. / -vs[i][2];
                [w as f32 / 2. + f * vs[i][0] * iz, h as f32 / 2. - f * vs[i][1] * iz, iz, q.uv[i][0] * iz, q.uv[i][1] * iz]
            });
            for tri in [[0, 1, 2], [0, 2, 3]] {
                let [a, b, c] = tri.map(|i| s[i]);
                let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
                if area.abs() < 1e-6 {
                    continue;
                }
                let minx = a[0].min(b[0]).min(c[0]).floor().max(0.) as usize;
                let maxx = (a[0].max(b[0]).max(c[0]).ceil().max(0.) as usize).min(w);
                let miny = a[1].min(b[1]).min(c[1]).floor().max(0.) as usize;
                let maxy = (a[1].max(b[1]).max(c[1]).ceil().max(0.) as usize).min(h);
                for py in miny..maxy {
                    for px in minx..maxx {
                        let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
                        let w0 = ((b[0] - x) * (c[1] - y) - (b[1] - y) * (c[0] - x)) / area;
                        let w1 = ((c[0] - x) * (a[1] - y) - (c[1] - y) * (a[0] - x)) / area;
                        let w2 = 1. - w0 - w1;
                        if w0 < 0. || w1 < 0. || w2 < 0. {
                            continue;
                        }
                        let iz = w0 * a[2] + w1 * b[2] + w2 * c[2];
                        let i = py * w + px;
                        if iz <= depth[i] {
                            continue;
                        }
                        let [rx, ry, rw, rh] = q.rect;
                        let u = ((w0 * a[3] + w1 * b[3] + w2 * c[3]) / iz).floor().clamp(rx as f32, (rx + rw - 1) as f32) as u32;
                        let v = ((w0 * a[4] + w1 * b[4] + w2 * c[4]) / iz).floor().clamp(ry as f32, (ry + rh - 1) as f32) as u32;
                        let t = m.img.get_pixel(u, v).0;
                        let al = if q.overlay { t[3] as f32 / 255. } else { 1. };
                        if al <= 0. {
                            continue;
                        }
                        let o = &mut color[i * 4..i * 4 + 4];
                        for (k, ch) in [2, 1, 0].into_iter().enumerate() {
                            o[k] = (t[ch] as f32 * shade * tint[ch] * al + o[k] as f32 * (1. - al)).min(255.) as u8;
                        }
                        o[3] = (255. * al + o[3] as f32 * (1. - al)) as u8;
                        depth[i] = iz;
                        if m.pick {
                            pick[i] = 1 + ((q.overlay as u32) << 12) + v * 64 + u;
                        } else {
                            pick[i] = 0;
                        }
                    }
                }
            }
        }
    }
    Frame { w, color, pick }
}

/// Extrudes a sprite into voxels: pixel (x, y) sits at o + x*du + y*dv, `depth` is the thickness.
pub fn extrude(img: &RgbaImage, o: V3, du: V3, dv: V3, depth: V3, part: u8) -> Vec<Quad> {
    let (w, h) = img.dimensions();
    let opaque = |x: i64, y: i64| x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h && img.get_pixel(x as u32, y as u32)[3] > 0;
    let (n1, n2, n3) = (norm(du), norm(dv), norm(depth));
    let mut out = vec![];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            if !opaque(x, y) {
                continue;
            }
            let a = add(add(o, mul(du, x as f32)), mul(dv, y as f32));
            let rect = [x as u32, y as u32, 1, 1];
            let mut q = |o: V3, e1: V3, e2: V3, n: V3| out.push(face_quad(&Face { rect, o, du: e1, dv: e2, n }, part as usize, true, true));
            q(add(a, depth), du, dv, n3);
            q(a, dv, du, mul(n3, -1.));
            if !opaque(x - 1, y) { q(a, depth, dv, mul(n1, -1.)); }
            if !opaque(x + 1, y) { q(add(a, du), dv, depth, n1); }
            if !opaque(x, y - 1) { q(a, du, depth, mul(n2, -1.)); }
            if !opaque(x, y + 1) { q(add(a, dv), depth, du, n2); }
        }
    }
    out
}

/// Base-layout faces inflated by `inflate` (armor drawn from a texture in the skin layout).
pub fn shell(slim: bool, inflate: impl Fn(usize) -> f32) -> Vec<Quad> {
    let mut out = vec![];
    for (i, p) in skin::parts(slim).iter().enumerate() {
        let inf = inflate(i);
        let min = [p.min[0] - inf, p.min[1] - inf, p.min[2] - inf];
        let max = [p.min[0] + p.size[0] as f32 + inf, p.min[1] + p.size[1] as f32 + inf, p.min[2] + p.size[2] as f32 + inf];
        out.extend(skin::box_faces(min, max, p.uv, p.size).iter().map(|f| face_quad(f, i, true, false)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_view_picks_face_texel() {
        let img = skin::default_skin();
        let opts = ModelOpts { slim: false, base: true, overlay: false, voxels: false, thickness: 1. };
        let q = build(&img, &opts);
        let cam = Camera { yaw: 0., pitch: 0., dist: 60., fov: 40., target: [0., 28., 0.] };
        let fr = render(&q, &img, false, &[[0.; 3]; 6], &cam, 200, 200, [0, 0, 0, 0], true);
        // Centre of screen looks at the middle of the head front (texels 8..16, 8..16).
        let (u, v, ov) = fr.pick_at(100, 100).unwrap();
        assert!((8..16).contains(&u) && (8..16).contains(&v) && !ov, "{u} {v}");
        assert!(fr.pick_at(0, 0).is_none());
    }

    #[test]
    fn extrude_and_second_model_does_not_pick() {
        let mut spr = RgbaImage::new(2, 1);
        spr.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        spr.put_pixel(1, 0, image::Rgba([255, 0, 0, 255]));
        assert_eq!(extrude(&spr, [0.; 3], [1., 0., 0.], [0., -1., 0.], [0., 0., 1.], 0).len(), 10);
        let img = skin::default_skin();
        let q = build(&img, &ModelOpts { slim: false, base: true, overlay: false, voxels: false, thickness: 1. });
        let cam = Camera { yaw: 0., pitch: 0., dist: 60., fov: 40., target: [0., 28., 0.] };
        let m = Model { quads: &q, img: &img, slim: false, offset: [0.; 3], pose: [[0.; 3]; 6], explode: 0., pick: false };
        let fr = render_scene(&[m], &cam, 100, 100, [0; 4], [0; 4], Light::Minecraft);
        assert!(fr.pick_at(50, 50).is_none());
        assert!(fr.color[(50 * 100 + 50) * 4 + 3] > 0);
    }

    #[test]
    fn voxel_layer_builds_cubes_for_opaque_texels() {
        let mut img = RgbaImage::new(64, 64);
        img.put_pixel(40, 8, image::Rgba([255, 0, 0, 255])); // head overlay front, top-left
        let opts = ModelOpts { slim: false, base: false, overlay: true, voxels: true, thickness: 1. };
        assert_eq!(build(&img, &opts).len(), 5);
    }

    /// Dev helper: SKIN=in.png OUT=out.png cargo test render_dump -- --ignored
    #[test]
    #[ignore]
    fn render_dump() {
        let img = skin::load(&std::fs::read(std::env::var("SKIN").unwrap()).unwrap()).unwrap();
        let slim = skin::detect_slim(&img);
        let mut out = RgbaImage::new(400 * 2, 400);
        for (k, voxels) in [false, true].into_iter().enumerate() {
            let opts = ModelOpts { slim, base: true, overlay: true, voxels, thickness: 1. };
            let cam = Camera { yaw: 0.6, pitch: 0.3, dist: 55., fov: 35., target: [0., 16., 0.] };
            let fr = render(&build(&img, &opts), &img, slim, &walk_pose(1., 0.6), &cam, 400, 400, [30, 31, 36, 255], true);
            for (i, px) in fr.color.chunks(4).enumerate() {
                out.put_pixel((k * 400 + i % 400) as u32, (i / 400) as u32, image::Rgba([px[2], px[1], px[0], 255]));
            }
        }
        out.save(std::env::var("OUT").unwrap()).unwrap();
    }

    /// Regenerates the app icon: cargo test make_icon -- --ignored
    #[test]
    #[ignore]
    fn make_icon() {
        let n = 256u32;
        let mut img = skin::default_skin();
        // A little outer-layer fringe so the icon shows off the 3D layers.
        for x in 40..48 {
            img.put_pixel(x, 8, image::Rgba([90, 60, 34, 255]));
        }
        let opts = ModelOpts { slim: false, base: true, overlay: true, voxels: true, thickness: 1. };
        let cam = Camera { yaw: 0.62, pitch: 0.42, dist: 31., fov: 35., target: [0., 26.6, 0.] };
        let fr = render(&build(&img, &opts), &img, false, &[[0.; 3]; 6], &cam, n as usize, n as usize, [0, 0, 0, 0], true);
        let mut out = RgbaImage::new(n, n);
        let r = 56.;
        for (x, y, p) in out.enumerate_pixels_mut() {
            // Rounded tile with a vertical green gradient.
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let (cx, cy) = (fx.clamp(r, n as f32 - r), fy.clamp(r, n as f32 - r));
            let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            let a = (r - d + 0.5).clamp(0., 1.);
            let k = fy / n as f32;
            let bg = [(92. - 40. * k) as u8, (190. - 60. * k) as u8, (96. - 40. * k) as u8];
            let i = ((y * n + x) * 4) as usize;
            let (fa, fb) = (fr.color[i + 3] as f32 / 255., [fr.color[i + 2], fr.color[i + 1], fr.color[i]]);
            let c = [0, 1, 2].map(|j| (fb[j] as f32 + bg[j] as f32 * (1. - fa)) as u8);
            *p = image::Rgba([c[0], c[1], c[2], (a * 255.) as u8]);
        }
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/data/icons/hicolor/256x256/apps");
        std::fs::create_dir_all(dir).unwrap();
        out.save(format!("{dir}/io.github.skinvas.png")).unwrap();
    }
}
