// Resource packs built from the current skin: totem, paintings, default player skins.
use crate::editor::surface;
use crate::i18n::t;
use crate::{render, skin, tools};
use crate::AppCtx;
use adw::prelude::*;
use gtk::{cairo, gio};
use image::RgbaImage;
use std::{io::Write, path::PathBuf, rc::Rc};

/// (label, pack_format). 0 = every version 1.14+.
pub const VERSIONS: [(&str, u32); 17] = [
    ("1.14 – 1.21.10+", 0),
    ("1.21.9 – 1.21.10", 69),
    ("1.21.7 – 1.21.8", 64),
    ("1.21.6", 63),
    ("1.21.5", 55),
    ("1.21.4", 46),
    ("1.21.2 – 1.21.3", 42),
    ("1.21 – 1.21.1", 34),
    ("1.20.5 – 1.20.6", 32),
    ("1.20.3 – 1.20.4", 22),
    ("1.20.2", 18),
    ("1.20 – 1.20.1", 15),
    ("1.19.4", 13),
    ("1.19.3", 12),
    ("1.19 – 1.19.2", 9),
    ("1.18", 8),
    ("1.15 – 1.17", 5),
];

const DEFAULT_SKINS: [&str; 9] = ["alex", "ari", "efe", "kai", "makena", "noor", "steve", "sunny", "zuri"];

/// Front view sprite, 16x32 (base + outer layer), like the player seen from the front.
pub fn front(img: &RgbaImage, slim: bool) -> RgbaImage {
    let mut out = RgbaImage::new(16, 32);
    let parts = skin::parts(slim);
    for overlay in [false, true] {
        for p in &parts {
            let [rx, ry, rw, rh] = skin::part_faces(p, overlay)[1].rect;
            let (ox, oy) = ((p.min[0] + 8.) as u32, (32. - p.min[1] - p.size[1] as f32) as u32);
            for y in 0..rh {
                for x in 0..rw {
                    let c = img.get_pixel(rx + x, ry + y);
                    if c[3] > 0 {
                        out.put_pixel(ox + x, oy + y, *c);
                    }
                }
            }
        }
    }
    out
}

/// 16x16 totem: full head, body squashed to 5 rows, legs to 3.
pub fn totem(img: &RgbaImage, slim: bool) -> RgbaImage {
    let f = front(img, slim);
    RgbaImage::from_fn(16, 16, |x, y| {
        let sy = match y {
            0..8 => y,
            8..13 => 8 + (y - 8) * 12 / 5,
            _ => 20 + (y - 13) * 4,
        };
        *f.get_pixel(x, sy)
    })
}

fn head(img: &RgbaImage, slim: bool, scale: u32) -> RgbaImage {
    let f = image::imageops::crop_imm(&front(img, slim), 4, 0, 8, 8).to_image();
    image::imageops::resize(&f, 8 * scale, 8 * scale, image::imageops::FilterType::Nearest)
}

/// Renders the skin with a transparent background.
pub fn render_png(img: &RgbaImage, slim: bool, w: u32, h: u32, cam: render::Camera) -> RgbaImage {
    let q = render::build(img, &render::ModelOpts { slim, base: true, overlay: true, voxels: true, thickness: 1. });
    let fr = render::render(&q, img, slim, &[[0.; 3]; 6], &cam, w as usize, h as usize, [0; 4], true);
    crate::dialogs::frame_to_image(&fr.color, w, h)
}

/// Pack icon: the head in 3D.
pub fn head3d(img: &RgbaImage, slim: bool, size: u32) -> RgbaImage {
    render_png(img, slim, size, size, render::Camera { yaw: 0.62, pitch: 0.42, dist: 28., fov: 35., target: [0., 28., 0.] })
}

/// White banner shows the character: the banner "base" pattern is tinted by the dye, white = untouched.
pub fn banner(img: &RgbaImage, slim: bool) -> RgbaImage {
    let f = front(img, slim);
    let mut out = RgbaImage::new(64, 64);
    for (ox, oy) in [(1, 1), (22, 1)] {
        for y in 0..40 {
            for x in 0..20 {
                // Character centred on the 20x40 flag, with a 2px margin.
                let (sx, sy) = (x as i32 - 2, y as i32 * 32 / 40);
                let c = if (0..16).contains(&sx) { *f.get_pixel(sx as u32, sy as u32) } else { image::Rgba([0; 4]) };
                out.put_pixel(ox + x, oy + y, if c[3] > 0 { c } else { image::Rgba([235, 235, 235, 255]) });
            }
        }
    }
    out
}

/// Leather helmet overlay (untinted) that is your head: base + hat layer in armor layout.
pub fn helmet(img: &RgbaImage) -> RgbaImage {
    let mut out = RgbaImage::new(64, 32);
    image::imageops::replace(&mut out, &image::imageops::crop_imm(img, 0, 0, 64, 16).to_image(), 0, 0);
    out
}

/// Six 512px cube faces: sky + grass, with the character standing on face 0.
pub fn panorama(img: &RgbaImage, slim: bool) -> Vec<RgbaImage> {
    let n = 512;
    let sky = |y: u32| {
        let k = y as f32 / n as f32;
        image::Rgba([(111. + 96. * k) as u8, (168. + 59. * k) as u8, 255, 255])
    };
    let mut faces = vec![];
    for i in 0..6 {
        let mut f = RgbaImage::from_fn(n, n, |x, y| match i {
            4 => image::Rgba([111, 168, 255, 255]),
            5 => image::Rgba([86, 140, 60, 255]),
            _ if y > n * 3 / 5 => image::Rgba([86 + ((x * 7 + y * 13) % 9) as u8, 140 + ((x * 5 + y * 3) % 11) as u8, 60, 255]),
            _ => sky(y),
        });
        if i == 0 {
            let ch = render_png(img, slim, n, n, render::Camera { yaw: 0.35, pitch: 0.05, dist: 55., fov: 50., target: [0., 14., 0.] });
            image::imageops::overlay(&mut f, &ch, 0, 0);
        }
        faces.push(f);
    }
    faces
}

/// Foldable paper figure on an A4 sheet at 300 dpi (1 texel = 34 px ≈ 2.9 mm).
pub fn papercraft(img: &RgbaImage, slim: bool) -> RgbaImage {
    const S: u32 = 34;
    let mut page = RgbaImage::from_pixel(2480, 3508, image::Rgba([255, 255, 255, 255]));
    // Base with the outer layer flattened on top.
    let mut flat = img.clone();
    for p in skin::parts(slim) {
        for (b, o) in skin::part_faces(&p, false).iter().zip(skin::part_faces(&p, true)) {
            for y in 0..b.rect[3] {
                for x in 0..b.rect[2] {
                    let c = *img.get_pixel(o.rect[0] + x, o.rect[1] + y);
                    if c[3] > 0 {
                        flat.put_pixel(b.rect[0] + x, b.rect[1] + y, c);
                    }
                }
            }
        }
    }
    let parts = skin::parts(slim);
    // (part, x, y) in texels on the page; nets are (2w+2d) wide and (2d+h) tall.
    let place = [(skin::HEAD, 2, 2), (skin::BODY, 38, 2), (skin::RARM, 2, 30), (skin::LARM, 20, 30), (skin::RLEG, 38, 30), (skin::LLEG, 56, 30)];
    for (pi, px, py) in place {
        let p = &parts[pi];
        let [w, h, d] = p.size;
        let f = skin::part_faces(p, false);
        // Net: [right][front][left][back] in a row, top above front, bottom below front.
        let pos = [(0, d), (d, d), (d + w, d), (2 * d + w, d), (d, 0), (d, d + h)];
        for (k, (fx, fy)) in pos.iter().enumerate() {
            let [rx, ry, rw, rh] = f[k].rect;
            for y in 0..rh * S {
                for x in 0..rw * S {
                    let c = *flat.get_pixel(rx + x / S, ry + y / S);
                    let c = if c[3] == 0 { image::Rgba([235, 235, 235, 255]) } else { image::Rgba([c[0], c[1], c[2], 255]) };
                    let edge = x == 0 || y == 0 || x + 1 == rw * S || y + 1 == rh * S;
                    page.put_pixel((px + fx) * S + x, (py + fy) * S + y, if edge { image::Rgba([60, 60, 60, 255]) } else { c });
                }
            }
        }
        // Glue tabs on the sides of top and bottom, and on the back's free edge.
        let tab = S / 2;
        for (tx, ty, tw, th) in [((px + d) * S - tab, py * S, tab, d * S), ((px + d + w) * S, py * S, tab, d * S), ((px + d) * S - tab, (py + d + h) * S, tab, d * S), ((px + d + w) * S, (py + d + h) * S, tab, d * S), ((px + 2 * d + 2 * w) * S, (py + d) * S, tab, h * S)] {
            for y in ty..ty + th {
                for x in tx..tx + tw {
                    if x < 2480 && y < 3508 {
                        page.put_pixel(x, y, image::Rgba([210, 210, 210, 255]));
                    }
                }
            }
        }
    }
    page
}

/// Wavefront OBJ + MTL text for the model (1 block = 16 skin pixels).
pub fn obj(img: &RgbaImage, slim: bool, voxels: bool, name: &str) -> (String, String) {
    let q = render::build(img, &render::ModelOpts { slim, base: true, overlay: true, voxels, thickness: 1. });
    let mut o = format!("# Skinvas export\nmtllib {name}.mtl\nusemtl skin\n");
    for (i, quad) in q.iter().enumerate() {
        for v in quad.p {
            o += &format!("v {:.4} {:.4} {:.4}\n", v[0] / 16., v[1] / 16., v[2] / 16.);
        }
        for uv in quad.uv {
            o += &format!("vt {:.5} {:.5}\n", uv[0] / 64., 1. - uv[1] / 64.);
        }
        o += &format!("vn {} {} {}\n", quad.n[0], quad.n[1], quad.n[2]);
        let (b, n) = (i * 4 + 1, i + 1);
        o += &format!("f {b}/{b}/{n} {}/{}/{n} {}/{}/{n} {}/{}/{n}\n", b + 1, b + 1, b + 2, b + 2, b + 3, b + 3);
    }
    let mtl = format!("newmtl skin\nKd 1 1 1\nmap_Kd {name}.png\nmap_d {name}.png\n");
    (o, mtl)
}

fn save_bytes(ctx: &Rc<AppCtx>, name: &str, data: Vec<u8>) {
    let d = gtk::FileDialog::builder().modal(true).initial_name(name).build();
    let c = ctx.clone();
    d.save(Some(&ctx.win), None::<&gio::Cancellable>, move |r| {
        if let Some(p) = r.ok().and_then(|f| f.path()) {
            match std::fs::write(&p, &data) {
                Ok(()) => c.toast(&format!("{} {}", t("saved"), p.display())),
                Err(e) => c.toast(&format!("{}: {e}", t("error"))),
            }
        }
    });
}

fn export_group(ctx: &Rc<AppCtx>) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder().title(t("export")).description(t("export_tip")).build();
    type Make = fn(&RgbaImage, bool) -> RgbaImage;
    let items: [(&str, &str, Make); 3] = [
        ("export_avatar", "skinvas-avatar.png", |i, s| render_png(i, s, 512, 512, render::Camera { yaw: 0.5, pitch: 0.2, dist: 38., fov: 35., target: [0., 25., 0.] })),
        ("export_thumb", "skinvas-preview.png", |i, s| {
            let ch = render_png(i, s, 1280, 720, render::Camera { yaw: 0.45, pitch: 0.12, dist: 58., fov: 35., target: [-9., 16., 0.] });
            let mut bg = RgbaImage::from_fn(1280, 720, |x, y| image::Rgba([(30 + x / 40) as u8, (40 + y / 12) as u8, (70 + x / 20) as u8, 255]));
            image::imageops::overlay(&mut bg, &ch, 0, 0);
            bg
        }),
        ("export_paper", "skinvas-papercraft-A4.png", papercraft),
    ];
    for (key, file, make) in items {
        let row = adw::ActionRow::builder().title(t(key)).build();
        let b = gtk::Button::builder().label(t("save")).valign(gtk::Align::Center).build();
        let c = ctx.clone();
        b.connect_clicked(move |_| {
            let img = {
                let s = c.st.borrow();
                make(&s.skin, s.slim)
            };
            let d = gtk::FileDialog::builder().modal(true).initial_name(file).build();
            let c2 = c.clone();
            d.save(Some(&c.win), None::<&gio::Cancellable>, move |r| {
                if let Some(p) = r.ok().and_then(|f| f.path()) {
                    match img.save_with_format(&p, image::ImageFormat::Png) {
                        Ok(()) => c2.toast(&format!("{} {}", t("saved"), p.display())),
                        Err(e) => c2.toast(&format!("{}: {e}", t("error"))),
                    }
                }
            });
        });
        row.add_suffix(&b);
        g.add(&row);
    }
    // Bedrock skin pack.
    let row = adw::ActionRow::builder().title(t("export_mcpack")).subtitle(t("export_mcpack_tip")).build();
    let b = gtk::Button::builder().label(t("save")).valign(gtk::Align::Center).build();
    let c = ctx.clone();
    b.connect_clicked(move |_| {
        let (data, name) = {
            let s = c.st.borrow();
            let name = s.title().trim_end_matches(" •").trim_end_matches(".png").to_string();
            (tools::mcpack(&s.skin, s.slim, &name), name)
        };
        save_bytes(&c, &format!("{}.mcpack", file_name(&name).trim_end_matches(".zip")), data);
    });
    row.add_suffix(&b);
    g.add(&row);

    // 3D-printable STL with a chosen height.
    let row = adw::ActionRow::builder().title(t("export_stl")).subtitle(t("export_stl_tip")).build();
    let mm = gtk::SpinButton::with_range(20., 300., 5.);
    mm.set_value(80.);
    mm.set_valign(gtk::Align::Center);
    mm.set_tooltip_text(Some(t("height_mm")));
    let b = gtk::Button::builder().label(t("save")).valign(gtk::Align::Center).build();
    let (c, m2) = (ctx.clone(), mm.clone());
    b.connect_clicked(move |_| {
        let data = {
            let s = c.st.borrow();
            tools::stl(&s.skin, s.slim, m2.value() as f32)
        };
        save_bytes(&c, "skinvas-figure.stl", data);
    });
    row.add_suffix(&mm);
    row.add_suffix(&b);
    g.add(&row);

    let row = adw::ActionRow::builder().title(t("export_obj")).subtitle(t("export_obj_tip")).build();
    let b = gtk::Button::builder().label(t("save")).valign(gtk::Align::Center).build();
    let c = ctx.clone();
    b.connect_clicked(move |_| {
        let d = gtk::FileDialog::builder().modal(true).initial_name("skinvas.obj").build();
        let c2 = c.clone();
        d.save(Some(&c.win), None::<&gio::Cancellable>, move |r| {
            let Some(p) = r.ok().and_then(|f| f.path()) else { return };
            let p = p.with_extension("obj");
            let name = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            let s = c2.st.borrow();
            let (o, m) = obj(&s.skin, s.slim, s.voxels, &name);
            let r = std::fs::write(&p, o)
                .and_then(|_| std::fs::write(p.with_extension("mtl"), m))
                .map_err(|e| e.to_string())
                .and_then(|_| s.skin.save_with_format(p.with_extension("png"), image::ImageFormat::Png).map_err(|e| e.to_string()));
            drop(s);
            match r {
                Ok(()) => c2.toast(&format!("{} {}", t("saved"), p.display())),
                Err(e) => c2.toast(&format!("{}: {e}", t("error"))),
            }
        });
    });
    row.add_suffix(&b);
    g.add(&row);
    g
}

fn png(img: &RgbaImage) -> Vec<u8> {
    let mut b = std::io::Cursor::new(Vec::new());
    img.write_to(&mut b, image::ImageFormat::Png).unwrap();
    b.into_inner()
}

pub fn mcmeta(format: u32, desc: &str) -> String {
    let desc = serde_json::to_string(desc).unwrap();
    match format {
        // 1.21.9+ reads min/max_format, older versions read pack_format/supported_formats.
        0 => format!(r#"{{"pack":{{"pack_format":34,"supported_formats":[4,999],"min_format":4,"max_format":999,"description":{desc}}}}}"#),
        f if f >= 65 => format!(r#"{{"pack":{{"pack_format":{f},"min_format":{f},"max_format":{f},"description":{desc}}}}}"#),
        f => format!(r#"{{"pack":{{"pack_format":{f},"description":{desc}}}}}"#),
    }
}

pub struct Opts {
    pub totem: bool,
    pub paintings: bool,
    pub default_skin: bool,
    pub banner: bool,
    pub helmet: bool,
    pub panorama: bool,
    pub pbr: bool,
    pub format: u32,
    pub desc: String,
}

pub fn build_zip(img: &RgbaImage, slim: bool, o: &Opts) -> Vec<u8> {
    let mut files: Vec<(String, Vec<u8>)> = vec![("pack.mcmeta".into(), mcmeta(o.format, &o.desc).into_bytes()), ("pack.png".into(), png(&head3d(img, slim, 256)))];
    if o.totem {
        files.push(("assets/minecraft/textures/item/totem_of_undying.png".into(), png(&totem(img, slim))));
    }
    if o.paintings {
        let p = "assets/minecraft/textures/painting";
        files.push((format!("{p}/kebab.png"), png(&head(img, slim, 2))));
        files.push((format!("{p}/wanderer.png"), png(&front(img, slim))));
    }
    if o.banner {
        files.push(("assets/minecraft/textures/entity/banner/base.png".into(), png(&banner(img, slim))));
    }
    if o.helmet {
        let h = png(&helmet(img));
        files.push(("assets/minecraft/textures/models/armor/leather_layer_1_overlay.png".into(), h.clone()));
        files.push(("assets/minecraft/textures/entity/equipment/humanoid/leather_overlay.png".into(), h));
    }
    if o.panorama {
        for (i, face) in panorama(img, slim).iter().enumerate() {
            files.push((format!("assets/minecraft/textures/gui/title/background/panorama_{i}.png"), png(face)));
        }
    }
    if o.default_skin || o.pbr {
        let s = png(img);
        // LabPBR maps sit next to each default-skin texture as name_n.png / name_s.png.
        let maps = o.pbr.then(|| (png(&tools::normal_map(img, slim)), png(&tools::specular_map(img))));
        let mut paths = vec!["assets/minecraft/textures/entity/steve".to_string()];
        for model in ["wide", "slim"] {
            paths.extend(DEFAULT_SKINS.iter().map(|n| format!("assets/minecraft/textures/entity/player/{model}/{n}")));
        }
        for p in paths {
            files.push((format!("{p}.png"), s.clone()));
            if let Some((n, sp)) = &maps {
                files.push((format!("{p}_n.png"), n.clone()));
                files.push((format!("{p}_s.png"), sp.clone()));
            }
        }
    }
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in files {
        z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(&data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

/// resourcepacks folders of installed launchers (only ones that exist).
pub fn launcher_dirs() -> Vec<(String, PathBuf)> {
    let home = gtk::glib::home_dir();
    let mut out = vec![];
    let mc = home.join(".minecraft/resourcepacks");
    if mc.parent().is_some_and(|p| p.is_dir()) {
        out.push(("Minecraft / TLauncher (~/.minecraft)".into(), mc));
    }
    let roots = [
        ("Prism", ".local/share/PrismLauncher/instances"),
        ("Prism (Flatpak)", ".var/app/org.prismlauncher.PrismLauncher/data/PrismLauncher/instances"),
        ("MultiMC", ".local/share/multimc/instances"),
        ("Modrinth", ".local/share/ModrinthApp/profiles"),
    ];
    for (label, rel) in roots {
        let Ok(rd) = std::fs::read_dir(home.join(rel)) else { continue };
        let mut found: Vec<_> = rd.flatten().filter(|e| e.path().is_dir()).collect();
        found.sort_by_key(|e| e.file_name());
        for e in found {
            let p = e.path();
            // Prism/MultiMC keep the game in ".minecraft" or "minecraft"; Modrinth profiles are the game dir.
            let game = [p.join(".minecraft"), p.join("minecraft"), p.clone()].into_iter().find(|g| g.join("options.txt").exists() || g.join("saves").is_dir() || g.join("resourcepacks").is_dir());
            if let Some(g) = game {
                out.push((format!("{label}: {}", e.file_name().to_string_lossy()), g.join("resourcepacks")));
            }
        }
    }
    out
}

fn file_name(name: &str) -> String {
    let n: String = name.chars().map(|c| if c.is_alphanumeric() || " -_.()".contains(c) { c } else { '_' }).collect();
    let n = n.trim();
    format!("{}.zip", if n.is_empty() { "Skinvas" } else { n })
}

/// Never overwrite somebody else's pack: "name.zip", "name (2).zip", ...
fn free_path(dir: &std::path::Path, file: &str) -> PathBuf {
    let stem = file.trim_end_matches(".zip");
    (1..).map(|i| dir.join(if i == 1 { file.to_string() } else { format!("{stem} ({i}).zip") })).find(|p| !p.exists()).unwrap()
}

fn preview(ctx: &Rc<AppCtx>, size: (i32, i32), make: fn(&RgbaImage, bool) -> RgbaImage) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().content_width(size.0).content_height(size.1).halign(gtk::Align::Center).build();
    let st = ctx.st.clone();
    area.set_draw_func(move |_, cr, w, h| {
        let s = st.borrow();
        let img = make(&s.skin, s.slim);
        let sc = (w as f64 / img.width() as f64).min(h as f64 / img.height() as f64);
        cr.scale(sc, sc);
        let _ = cr.set_source_surface(surface(&img), 0., 0.);
        cr.source().set_filter(cairo::Filter::Nearest);
        let _ = cr.paint();
    });
    let a = area.clone();
    ctx.st.borrow_mut().listeners.push(Box::new(move |_| a.queue_draw()));
    area
}

pub fn page(ctx: &Rc<AppCtx>) -> gtk::Widget {
    let page = adw::PreferencesPage::new();

    let g = adw::PreferencesGroup::builder().title(t("preview")).build();
    let row = gtk::Box::builder().spacing(32).halign(gtk::Align::Center).margin_top(8).margin_bottom(8).build();
    for (label, area) in [(t("totem"), preview(ctx, (128, 128), totem)), (t("paintings"), preview(ctx, (64, 128), front))] {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
        b.append(&area);
        b.append(&gtk::Label::new(Some(label)));
        row.append(&b);
    }
    g.add(&row);
    page.add(&g);

    let g = adw::PreferencesGroup::builder().title(t("pack_content")).build();
    let sw = |key: &str, sub: &str, on: bool| {
        let r = adw::SwitchRow::builder().title(t(key)).subtitle(t(sub)).active(on).build();
        g.add(&r);
        r
    };
    let totem_r = sw("totem", "totem_tip", true);
    let paint_r = sw("paintings", "paintings_tip", false);
    let def_r = sw("default_skin", "default_skin_tip", false);
    let banner_r = sw("banner", "banner_tip", false);
    let helmet_r = sw("helmet", "helmet_tip", false);
    let pano_r = sw("panorama", "panorama_tip", false);
    let pbr_r = sw("pbr", "pbr_tip", false);
    page.add(&g);

    let g = adw::PreferencesGroup::builder().title(t("pack")).build();
    let name_r = adw::EntryRow::builder().title(t("pack_name")).text("Skinvas Totem").build();
    let desc_r = adw::EntryRow::builder().title(t("pack_desc")).text(t("pack_desc_default")).build();
    let labels: Vec<&str> = VERSIONS.iter().map(|v| v.0).collect();
    let ver_r = adw::ComboRow::builder().title(t("mc_version")).model(&gtk::StringList::new(&labels)).build();
    g.add(&name_r);
    g.add(&desc_r);
    g.add(&ver_r);
    page.add(&g);

    let make = {
        let (ctx, totem_r, paint_r, def_r, desc_r, ver_r) = (ctx.clone(), totem_r.clone(), paint_r.clone(), def_r.clone(), desc_r.clone(), ver_r.clone());
        let (banner_r, helmet_r, pano_r, pbr_r) = (banner_r.clone(), helmet_r.clone(), pano_r.clone(), pbr_r.clone());
        move || {
            let o = Opts { totem: totem_r.is_active(), paintings: paint_r.is_active(), default_skin: def_r.is_active(), banner: banner_r.is_active(), helmet: helmet_r.is_active(), panorama: pano_r.is_active(), pbr: pbr_r.is_active(), format: VERSIONS[ver_r.selected() as usize].1, desc: desc_r.text().into() };
            let s = ctx.st.borrow();
            build_zip(&s.skin, s.slim, &o)
        }
    };
    let make = Rc::new(make);

    let g = adw::PreferencesGroup::builder().title(t("install")).build();
    let dirs = launcher_dirs();
    if dirs.is_empty() {
        g.set_description(Some(t("no_launchers")));
    } else {
        let labels: Vec<&str> = dirs.iter().map(|d| d.0.as_str()).collect();
        let dir_r = adw::ComboRow::builder().title(t("install_to")).model(&gtk::StringList::new(&labels)).build();
        let btn = gtk::Button::builder().label(t("install")).valign(gtk::Align::Center).css_classes(["suggested-action"]).build();
        dir_r.add_suffix(&btn);
        let (c, m, n, d) = (ctx.clone(), make.clone(), name_r.clone(), dir_r.clone());
        btn.connect_clicked(move |_| {
            let dir = &dirs[d.selected() as usize].1;
            let path = free_path(dir, &file_name(&n.text()));
            match std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&path, m())) {
                Ok(()) => {
                    c.st.borrow_mut().emote(1);
                    c.toast(&format!("{} {}", t("installed"), path.display()))
                }
                Err(e) => c.toast(&format!("{}: {e}", t("error"))),
            }
        });
        g.add(&dir_r);
    }
    let save = gtk::Button::builder().label(t("save_zip")).halign(gtk::Align::Center).margin_top(12).build();
    let (c, m, n) = (ctx.clone(), make.clone(), name_r.clone());
    save.connect_clicked(move |_| {
        let d = gtk::FileDialog::builder().modal(true).initial_name(file_name(&n.text())).build();
        let (c2, m2) = (c.clone(), m.clone());
        d.save(Some(&c.win), None::<&gio::Cancellable>, move |r| {
            if let Some(p) = r.ok().and_then(|f| f.path()) {
                match std::fs::write(&p, m2()) {
                    Ok(()) => c2.toast(t("saved")),
                    Err(e) => c2.toast(&format!("{}: {e}", t("error"))),
                }
            }
        });
    });
    g.add(&save);
    page.add(&g);
    page.add(&export_group(ctx));
    page.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totem_and_front_layout() {
        let img = skin::default_skin();
        let f = front(&img, false);
        assert_eq!(f.dimensions(), (16, 32));
        assert_eq!(f.get_pixel(4 + 1, 4), img.get_pixel(8 + 1, 8 + 4)); // head front eye row
        assert_eq!(f.get_pixel(0, 31)[3], 0); // below the arms is empty
        assert_eq!(totem(&img, false).get_pixel(5, 4), f.get_pixel(5, 4));
        assert_eq!(totem(&img, false).get_pixel(5, 15), f.get_pixel(5, 28));
    }

    #[test]
    fn zip_contains_requested_files() {
        let o = Opts { totem: true, paintings: false, default_skin: true, banner: true, helmet: true, panorama: true, pbr: true, format: 69, desc: "a\"b".into() };
        let z = zip::ZipArchive::new(std::io::Cursor::new(build_zip(&skin::default_skin(), false, &o))).unwrap();
        let names: Vec<String> = z.file_names().flatten().map(String::from).collect();
        let names: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        assert!(names.contains(&"assets/minecraft/textures/item/totem_of_undying.png"));
        assert!(names.contains(&"assets/minecraft/textures/entity/player/slim/alex.png"));
        assert!(!names.iter().any(|n| n.contains("painting")));
        assert!(names.contains(&"assets/minecraft/textures/gui/title/background/panorama_5.png"));
        assert!(names.contains(&"assets/minecraft/textures/entity/banner/base.png"));
        let (o, m) = obj(&skin::default_skin(), false, false, "x");
        assert_eq!(o.matches("\nf ").count(), 72); // 6 parts x 2 layers x 6 faces
        assert!(m.contains("map_Kd x.png"));
        let paper = papercraft(&skin::default_skin(), false);
        assert_eq!(paper.dimensions(), (2480, 3508));
        assert_ne!(paper.get_pixel(2 * 34 + 8 * 34 + 10, 2 * 34 + 8 * 34 + 10).0, [255, 255, 255, 255]); // head front is printed
        let meta: serde_json::Value = serde_json::from_str(&mcmeta(69, "a\"b")).unwrap();
        assert_eq!(meta["pack"]["min_format"], 69);
        assert!(serde_json::from_str::<serde_json::Value>(&mcmeta(0, "x")).is_ok());
        assert_eq!(file_name("my/pack:1"), "my_pack_1.zip");
    }

    /// Dev helper: SKIN=in.png OUT=out.png cargo test totem_dump -- --ignored
    #[test]
    #[ignore]
    fn totem_dump() {
        let img = skin::load(&std::fs::read(std::env::var("SKIN").unwrap()).unwrap()).unwrap();
        let slim = skin::detect_slim(&img);
        let mut out = RgbaImage::from_pixel(16 * 8 + 16 * 8 + 8, 32 * 8, image::Rgba([40, 40, 46, 255]));
        let big = |i: &RgbaImage| image::imageops::resize(i, i.width() * 8, i.height() * 8, image::imageops::FilterType::Nearest);
        image::imageops::overlay(&mut out, &big(&totem(&img, slim)), 0, 0);
        image::imageops::overlay(&mut out, &big(&front(&img, slim)), 16 * 8 + 8, 0);
        out.save(std::env::var("OUT").unwrap()).unwrap();
    }
}
