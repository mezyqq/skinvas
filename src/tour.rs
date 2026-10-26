// First-run welcome: language, theme, account, then every feature shown with a live animation.
use crate::editor::{surface, State};
use crate::i18n::{t, LANGS};
use crate::render::{self, Camera, Light, Model, ModelOpts};
use crate::settings::{parse_hex, Settings};
use crate::settings_page::{commit, ACCENTS};
use crate::{skin, tools, AppCtx};
use adw::prelude::*;
use gtk::{cairo, glib};
use image::{Rgba, RgbaImage};
use std::{
    cell::{Cell, RefCell},
    f64::consts::PI,
    rc::Rc,
};

fn ease_out(x: f64) -> f64 {
    1. - (1. - x.clamp(0., 1.)).powi(3)
}


/// A random character with a real outer layer (hair, jacket, sleeves) so 3D layers have something to show.
pub fn demo_skin(seed: u64) -> RgbaImage {
    let mut img = tools::random_skin(seed);
    let mut r = tools::Rng::new(seed ^ 0x9e37);
    let ps = skin::parts(false);
    let jacket = {
        let [cr, cg, cb] = crate::colorpick::hsv_to_rgb(r.f() * 360., 0.55, 0.55);
        [(cr * 255.) as u8, (cg * 255.) as u8, (cb * 255.) as u8, 255]
    };
    for (i, p) in ps.iter().enumerate() {
        for (k, (b, o)) in skin::part_faces(p, false).iter().zip(skin::part_faces(p, true)).enumerate() {
            let [bx, by, w, h] = b.rect;
            let [ox, oy, _, _] = o.rect;
            for y in 0..h {
                for x in 0..w {
                    let src = *img.get_pixel(bx + x, by + y);
                    let put = match i {
                        skin::HEAD => k == 4 || (k != 5 && y < 2) || (k == 3 && y < 5),
                        skin::BODY => k != 4 && k != 5 && (k != 1 || x == 0 || x + 1 == w || x == w / 2),
                        skin::RARM | skin::LARM => k < 4 && y < 5,
                        _ => false,
                    };
                    if put {
                        let c = if i == skin::HEAD { src.0 } else { jacket };
                        let n = (r.f() - 0.5) * 30.;
                        let c = [0, 1, 2].map(|j| (c[j] as f64 + n).clamp(0., 255.) as u8);
                        img.put_pixel(ox + x, oy + y, Rgba([c[0], c[1], c[2], 255]));
                    }
                }
            }
        }
    }
    img
}

fn paint_frame(cr: &cairo::Context, fr: &render::Frame, x: f64, y: f64, w: usize, h: usize) {
    let mut data = fr.color.clone();
    data.truncate(w * h * 4);
    if let Ok(s) = cairo::ImageSurface::create_for_data(data, cairo::Format::ARgb32, w as i32, h as i32, w as i32 * 4) {
        let _ = cr.set_source_surface(&s, x, y);
        let _ = cr.paint();
    }
}

fn draw_model(cr: &cairo::Context, img: &RgbaImage, x: f64, y: f64, w: usize, h: usize, yaw: f32, walk: f32, thick: f32, voxels: bool) {
    let q = render::build(img, &ModelOpts { slim: false, base: true, overlay: true, voxels, thickness: thick });
    let m = Model { quads: &q, img, slim: false, offset: [0.; 3], pose: render::walk_pose(walk, 0.6), explode: 0., pick: false };
    let cam = Camera { yaw, pitch: 0.18, dist: 62., fov: 35., target: [0., 16., 0.] };
    paint_frame(cr, &render::render_scene(&[m], &cam, w, h, [0; 4], [0; 4], Light::Soft), x, y, w, h);
}

fn text(cr: &cairo::Context, s: &str, x: f64, y: f64, size: f64, bold: bool, rgba: [f64; 4]) {
    cr.select_font_face("Sans", cairo::FontSlant::Normal, if bold { cairo::FontWeight::Bold } else { cairo::FontWeight::Normal });
    cr.set_font_size(size);
    cr.set_source_rgba(rgba[0], rgba[1], rgba[2], rgba[3]);
    cr.move_to(x, y);
    let _ = cr.show_text(s);
}

fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2., 0.);
    cr.arc(x + w - r, y + h - r, r, 0., PI / 2.);
    cr.arc(x + r, y + h - r, r, PI / 2., PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

/// Shared timing: which page is visible and when each page was entered (animations restart on entry).
#[derive(Clone)]
struct Clock {
    now: Rc<Cell<f64>>,
    page: Rc<Cell<u32>>,
    entered: Rc<RefCell<Vec<f64>>>,
}

impl Clock {
    /// Seconds since the page was entered; an entry before the first frame is stamped lazily.
    fn since(&self, page: u32) -> f64 {
        let now = self.now.get();
        let mut e = self.entered.borrow_mut();
        let Some(t0) = e.get_mut(page as usize) else { return 0. };
        if *t0 < 0. {
            *t0 = now;
        }
        now - *t0
    }
}

/// A DrawingArea that animates only while its page is on screen.
fn anim(clock: &Clock, page: u32, w: i32, h: i32, draw: impl Fn(&cairo::Context, f64, f64, f64) + 'static) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::builder().content_width(w).content_height(h).halign(gtk::Align::Center).build();
    let c = clock.clone();
    a.set_draw_func(move |_, cr, w, h| draw(cr, w as f64, h as f64, c.since(page)));
    let c = clock.clone();
    a.add_tick_callback(move |a, _| {
        if c.page.get() == page {
            a.queue_draw();
        }
        glib::ControlFlow::Continue
    });
    a
}

fn page_box(title: &str, body: &str, art: Option<&gtk::Widget>) -> (gtk::Box, gtk::Label, gtk::Label) {
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).valign(gtk::Align::Center).halign(gtk::Align::Center).hexpand(true).vexpand(true).build();
    if let Some(a) = art {
        b.append(a);
    }
    let tl = gtk::Label::builder().label(title).css_classes(["sv-tour-title"]).wrap(true).justify(gtk::Justification::Center).build();
    let bl = gtk::Label::builder().label(body).css_classes(["sv-tour-body"]).wrap(true).justify(gtk::Justification::Center).max_width_chars(60).build();
    b.append(&tl);
    b.append(&bl);
    (b, tl, bl)
}

/// Gentle fade-in of page content when the page appears.
fn reveal(w: &impl IsA<gtk::Widget>, delay: u32) {
    let w = w.clone().upcast::<gtk::Widget>();
    w.set_opacity(0.);
    glib::timeout_add_local_once(std::time::Duration::from_millis(delay as u64), move || {
        let a = adw::TimedAnimation::new(&w, 0., 1., 700, adw::PropertyAnimationTarget::new(&w, "opacity"));
        a.set_easing(adw::Easing::EaseOutCubic);
        a.play();
    });
}

pub fn show(ctx: &Rc<AppCtx>) {
    show_at(ctx, 0);
}

/// The edited skin (Steve on a fresh start) is the star of the tour.
fn hero(ctx: &Rc<AppCtx>) -> impl Fn() -> RgbaImage + 'static {
    let st = ctx.st.clone();
    move || st.borrow().skin.clone()
}

pub fn show_at(ctx: &Rc<AppCtx>, start: u32) {
    let clock = Clock { now: Rc::new(Cell::new(0.)), page: Rc::new(Cell::new(start)), entered: Rc::new(RefCell::new(vec![-1.; 16])) };
    let crowd: Rc<Vec<RgbaImage>> = Rc::new((0..6).map(|i| demo_skin(1000 + i * 77)).collect());
    let accent = parse_hex(&ctx.st.borrow().settings.accent);
    let acc = [accent[0] as f64 / 255., accent[1] as f64 / 255., accent[2] as f64 / 255.];
    // Slow, steady turn shared by every model so pages feel calm and consistent.
    const SPIN: f64 = 0.35;
    let car = adw::Carousel::builder().hexpand(true).vexpand(true).allow_scroll_wheel(false).build();
    let mut reveals: Vec<Vec<gtk::Widget>> = vec![];
    let mut add = |w: &gtk::Box, items: Vec<gtk::Widget>| {
        car.append(w);
        reveals.push(items);
    };

    // 0 — welcome.
    {
        let h = hero(ctx);
        let art = anim(&clock, 0, 280, 300, move |cr, w, hh, t| {
            cr.push_group();
            draw_model(cr, &h(), 0., 0., w as usize, hh as usize, (0.5 + t * SPIN) as f32, 0., 1., true);
            let _ = cr.pop_group_to_source();
            let _ = cr.paint_with_alpha(ease_out(t / 1.2));
        });
        let logo = gtk::Label::builder().use_markup(true).label(format!("Skin<span foreground='#{:02x}{:02x}{:02x}'>vas</span>", accent[0], accent[1], accent[2])).css_classes(["sv-tour-logo"]).build();
        let (b, tl, bl) = page_box(t("tw_title"), t("tw_body"), Some(art.upcast_ref()));
        b.insert_child_after(&logo, Some(&art));
        add(&b, vec![logo.upcast(), tl.upcast(), bl.upcast()]);
    }

    // 1 — language.
    {
        let grid = gtk::Box::builder().spacing(12).halign(gtk::Align::Center).build();
        let cur = ctx.st.borrow().settings.lang.clone();
        for (i, (code, name)) in LANGS.iter().enumerate() {
            let hello = ["Привет", "Hello", "Привіт", "Cześć"][i];
            let card = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).width_request(130).build();
            card.append(&gtk::Label::builder().label(hello).css_classes(["sv-tour-hello"]).build());
            card.append(&gtk::Label::builder().label(*name).css_classes(["dim-label"]).build());
            let btn = gtk::Button::builder().child(&card).css_classes(["sv-tour-card"]).build();
            if *code == cur {
                btn.add_css_class("selected");
            }
            let c = ctx.clone();
            let code = code.to_string();
            btn.connect_clicked(move |b| {
                crate::i18n::set_lang(&code);
                commit(&c, |s| s.lang = code.clone(), true);
                // Re-open on this page in the new language.
                let (c2, d) = (c.clone(), b.ancestor(adw::Dialog::static_type()).and_downcast::<adw::Dialog>());
                glib::idle_add_local_once(move || {
                    if let Some(d) = d {
                        unsafe { d.set_data("sv-reopen", true) };
                        d.close();
                    }
                    show_at(&c2, 1);
                });
            });
            grid.append(&btn);
        }
        let (b, tl, bl) = page_box(t("tl_title"), t("tl_body"), None);
        b.prepend(&grid);
        add(&b, vec![grid.upcast(), tl.upcast(), bl.upcast()]);
    }

    // 2 — theme + accent.
    {
        let row = gtk::Box::builder().spacing(16).halign(gtk::Align::Center).build();
        let cur = ctx.st.borrow().settings.theme.clone();
        let mut cards = vec![];
        for (key, dark) in [("theme_light", false), ("theme_dark", true)] {
            let mock = gtk::DrawingArea::builder().content_width(200).content_height(124).build();
            let st = ctx.st.clone();
            mock.set_draw_func(move |_, cr, w, h| {
                let a = parse_hex(&st.borrow().settings.accent);
                let a = [a[0] as f64 / 255., a[1] as f64 / 255., a[2] as f64 / 255.];
                let (bg, side, fg) = if dark { (0.13, 0.10, 0.9) } else { (0.97, 0.92, 0.15) };
                rounded(cr, 0., 0., w as f64, h as f64, 12.);
                cr.set_source_rgb(bg, bg, bg + 0.02);
                let _ = cr.fill();
                rounded(cr, 0., 0., 56., h as f64, 12.);
                cr.set_source_rgb(side, side, side + 0.02);
                let _ = cr.fill();
                rounded(cr, 8., 12., 40., 12., 6.);
                cr.set_source_rgb(a[0], a[1], a[2]);
                let _ = cr.fill();
                for i in 0..3 {
                    rounded(cr, 8., 32. + i as f64 * 16., 34., 7., 3.5);
                    cr.set_source_rgba(fg, fg, fg, 0.22);
                    let _ = cr.fill();
                }
                for i in 0..6 {
                    let (x, y) = (70. + (i % 3) as f64 * 42., 14. + (i / 3) as f64 * 54.);
                    rounded(cr, x, y, 34., 44., 7.);
                    cr.set_source_rgba(fg, fg, fg, 0.07);
                    let _ = cr.fill();
                }
            });
            let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
            col.append(&mock);
            col.append(&gtk::Label::new(Some(t(key))));
            let btn = gtk::Button::builder().child(&col).css_classes(["sv-tour-card"]).build();
            if (cur == "light") != dark {
                btn.add_css_class("selected");
            }
            cards.push((btn.clone(), mock));
            row.append(&btn);
        }
        let all: Vec<(gtk::Button, gtk::DrawingArea)> = cards.clone();
        for (i, (btn, _)) in cards.iter().enumerate() {
            let (c, all) = (ctx.clone(), all.clone());
            btn.connect_clicked(move |_| {
                commit(&c, |s| s.theme = if i == 0 { "light" } else { "dark" }.into(), false);
                for (j, (b, _)) in all.iter().enumerate() {
                    if i == j { b.add_css_class("selected") } else { b.remove_css_class("selected") }
                }
            });
        }
        let sw = gtk::Box::builder().spacing(10).halign(gtk::Align::Center).build();
        for (i, hex) in ACCENTS.iter().enumerate() {
            let b = gtk::Button::builder().css_classes(["sv-sw", &format!("sv-sw-{i}")]).build();
            b.set_size_request(28, 28);
            let (c, mocks) = (ctx.clone(), cards.iter().map(|c| c.1.clone()).collect::<Vec<_>>());
            b.connect_clicked(move |_| {
                commit(&c, |s| s.accent = hex.to_string(), false);
                mocks.iter().for_each(|m| m.queue_draw());
            });
            sw.append(&b);
        }
        let (b, tl, bl) = page_box(t("tt_title"), t("tt_body"), None);
        b.prepend(&sw);
        b.prepend(&row);
        add(&b, vec![row.upcast(), sw.upcast(), tl.upcast(), bl.upcast()]);
    }

    // 3 — Ely.by account (optional).
    {
        let card = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).width_request(340).css_classes(["sv-tour-panel"]).halign(gtk::Align::Center).build();
        let status = gtk::Label::builder().css_classes(["sv-tour-body"]).wrap(true).build();
        match crate::library::Account::load() {
            Some(a) => status.set_label(&format!("✓ {} {}", t("logged_in_as"), a.name)),
            None => {
                let user = gtk::Entry::builder().placeholder_text(t("login")).build();
                let pass = gtk::PasswordEntry::builder().placeholder_text(t("password")).show_peek_icon(true).build();
                let totp = gtk::Entry::builder().placeholder_text(t("totp")).build();
                let go = gtk::Button::builder().label(t("sign_in")).css_classes(["suggested-action", "pill"]).build();
                let form = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build();
                for w in [user.upcast_ref::<gtk::Widget>(), pass.upcast_ref(), totp.upcast_ref(), go.upcast_ref()] {
                    form.append(w);
                }
                card.append(&form);
                let st2 = status.clone();
                go.connect_clicked(move |b| {
                    let (u, code) = (user.text().trim().to_string(), totp.text().trim().to_string());
                    let p = if code.is_empty() { pass.text().to_string() } else { format!("{}:{code}", pass.text()) };
                    if u.is_empty() || p.is_empty() {
                        return;
                    }
                    b.set_sensitive(false);
                    let client = format!("{:032x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
                    let (b2, s2, f2) = (b.clone(), st2.clone(), form.clone());
                    crate::browser::fetch(
                        move || crate::net::ely_login(&u, &p, &client).map(|(token, name, uuid)| crate::library::Account { name, uuid, token, client_token: client }),
                        move |r| {
                            b2.set_sensitive(true);
                            match r.and_then(|a| a.save().map(|_| a)) {
                                Ok(a) => {
                                    f2.set_visible(false);
                                    s2.set_label(&format!("✓ {} {}", t("logged_in_as"), a.name));
                                    reveal(&s2, 0);
                                }
                                Err(e) => s2.set_label(&format!("{}: {e}", t("error"))),
                            }
                        },
                    );
                });
            }
        }
        card.append(&status);
        let (b, tl, bl) = page_box(t("ta_title"), t("ta_body"), None);
        b.prepend(&card);
        add(&b, vec![card.upcast(), tl.upcast(), bl.upcast()]);
    }

    // 4 — editor: the skin fills in on the layout and on the model at the same time.
    {
        let h = hero(ctx);
        let order: Rc<Vec<(u32, u32)>> = Rc::new(
            skin::parts(false)
                .iter()
                .flat_map(|p| [false, true].into_iter().flat_map(move |ov| skin::part_faces(p, ov)))
                .flat_map(|f| (0..f.rect[3]).flat_map(move |y| (0..f.rect[2]).map(move |x| (f.rect[0] + x, f.rect[1] + y))))
                .collect(),
        );
        let art = anim(&clock, 4, 600, 280, move |cr, w, hh, t| {
            let target = h();
            let k = ease_out(t / 6.);
            let n = (k * order.len() as f64) as usize;
            let mut img = RgbaImage::new(64, 64);
            for p in skin::parts(false) {
                for f in skin::part_faces(&p, false) {
                    skin::fill_rect(&mut img, f.rect, [128, 132, 140, 255]);
                }
            }
            for &(x, y) in &order[..n] {
                img.put_pixel(x, y, *target.get_pixel(x, y));
            }
            let sc = 4.;
            let (ox, oy) = (24., (hh - 64. * sc) / 2.);
            rounded(cr, ox - 8., oy - 8., 64. * sc + 16., 64. * sc + 16., 12.);
            cr.set_source_rgba(1., 1., 1., 0.05);
            let _ = cr.fill();
            cr.save().ok();
            cr.translate(ox, oy);
            cr.scale(sc, sc);
            let _ = cr.set_source_surface(surface(&img), 0., 0.);
            cr.source().set_filter(cairo::Filter::Nearest);
            let _ = cr.paint();
            cr.restore().ok();
            draw_model(cr, &img, w - 300., 0., 280, hh as usize, (0.5 + t * SPIN) as f32, 0., 1., true);
        });
        let (b, tl, bl) = page_box(t("te_title"), t("te_body"), Some(art.upcast_ref()));
        add(&b, vec![tl.upcast(), bl.upcast()]);
    }

    // 5 — 3D: the outer layer grows out once, then scenes change slowly with a soft crossfade.
    // The cat skin shows the voxel layer off best, so it's the star of this page.
    {
        let cat = skin::load(include_bytes!("../data/tour/cat.png")).unwrap_or_else(|_| skin::default_skin());
        let cat_slim = skin::detect_slim(&cat);
        let h = move || cat.clone();
        let art = anim(&clock, 5, 600, 300, move |cr, w, hh, t| {
            let mut s = State::new(Settings::default());
            s.skin = h();
            s.slim = cat_slim;
            s.settings.thickness = 0.05 + 0.95 * ease_out(t / 2.);
            let scenes = [(0u32, 0usize, 0usize), (2, 0, 0), (3, 0, 0)];
            let step = (t / 4.) as usize;
            let frac = ((t % 4.) / 0.8).min(1.);
            rounded(cr, 0., 0., w, hh, 18.);
            cr.clip();
            let mut s = s;
            let mut draw = |i: usize, alpha: f64| {
                let (sc, ar, it) = scenes[i % scenes.len()];
                (s.scene, s.armor, s.item) = (sc, ar, it);
                let fr = s.render_view(w as usize, hh as usize, (0.5 + t * SPIN) as f32, 0., None, true);
                cr.push_group();
                paint_frame(cr, &fr, 0., 0., w as usize, hh as usize);
                let _ = cr.pop_group_to_source();
                let _ = cr.paint_with_alpha(alpha);
            };
            if step > 0 && frac < 1. {
                draw(step - 1, 1.);
            }
            draw(step, if step == 0 { 1. } else { ease_out(frac) });
        });
        let (b, tl, bl) = page_box(t("t3_title"), t("t3_body"), Some(art.upcast_ref()));
        add(&b, vec![tl.upcast(), bl.upcast()]);
    }

    // 6 — skins: a calm grid of characters fading in.
    {
        let crowd = crowd.clone();
        let art = anim(&clock, 6, 600, 290, move |cr, _w, _h, t| {
            for i in 0..6 {
                let a = ease_out((t - i as f64 * 0.12) / 0.8);
                let (cw, ch) = (180., 135.);
                let (x, y) = (15. + (i % 3) as f64 * (cw + 15.), 5. + (i / 3) as f64 * (ch + 15.));
                cr.push_group();
                rounded(cr, x, y, cw, ch, 14.);
                cr.set_source_rgba(1., 1., 1., 0.06);
                let _ = cr.fill();
                draw_model(cr, &crowd[i], x + 35., y + 5., 110, 125, (0.5 + t * SPIN + i as f64 * 0.4) as f32, 0., 1., true);
                let _ = cr.pop_group_to_source();
                let _ = cr.paint_with_alpha(a);
            }
        });
        let (b, tl, bl) = page_box(t("tb_title"), t("tb_body"), Some(art.upcast_ref()));
        add(&b, vec![tl.upcast(), bl.upcast()]);
    }

    // 7 — packs & export: the totem floats gently, export formats underneath.
    {
        let h = hero(ctx);
        let art = anim(&clock, 7, 600, 290, move |cr, w, _hh, t| {
            let totem = crate::packs::totem(&h(), false);
            let (cx, cy) = (w / 2., 110. + (t * 1.2).sin() * 4.);
            let size = 150. * (0.9 + 0.1 * ease_out(t / 1.));
            cr.push_group();
            cr.save().ok();
            cr.translate(cx, cy);
            cr.scale(size / 16., size / 16.);
            let _ = cr.set_source_surface(surface(&totem), -8., -8.);
            cr.source().set_filter(cairo::Filter::Nearest);
            let _ = cr.paint();
            cr.restore().ok();
            let _ = cr.pop_group_to_source();
            let _ = cr.paint_with_alpha(ease_out(t / 1.));
            for (k, label) in ["ZIP", "GIF", "PNG", "OBJ", "A4"].iter().enumerate() {
                let x = w / 2. - 250. + k as f64 * 104.;
                let a = ease_out((t - 0.6 - k as f64 * 0.12) / 0.8);
                rounded(cr, x, 230., 86., 34., 17.);
                cr.set_source_rgba(1., 1., 1., 0.08 * a);
                let _ = cr.fill();
                text(cr, label, x + 27., 252., 14., true, [acc[0], acc[1], acc[2], a]);
            }
        });
        let (b, tl, bl) = page_box(t("tp_title"), t("tp_body"), Some(art.upcast_ref()));
        add(&b, vec![tl.upcast(), bl.upcast()]);
    }

    // 8 — done.
    let last = {
        let h = hero(ctx);
        let art = anim(&clock, 8, 240, 260, move |cr, w, hh, t| {
            draw_model(cr, &h(), 0., 0., w as usize, hh as usize, (0.5 + t * SPIN) as f32, 0., 1., true);
        });
        let (b, tl, bl) = page_box(t("td_title"), t("td_body"), Some(art.upcast_ref()));
        add(&b, vec![art.clone().upcast(), tl.upcast(), bl.upcast()]);
        car.n_pages() - 1
    };

    // Still background: deep base with a soft accent glow (no motion, so nothing pulls the eye).
    let bg = gtk::DrawingArea::builder().hexpand(true).vexpand(true).build();
    bg.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        cr.set_source_rgb(0.055, 0.06, 0.075);
        let _ = cr.paint();
        let g = cairo::RadialGradient::new(w * 0.5, h * 0.05, 0., w * 0.5, h * 0.05, w * 0.7);
        g.add_color_stop_rgba(0., acc[0], acc[1], acc[2], 0.18);
        g.add_color_stop_rgba(1., acc[0], acc[1], acc[2], 0.);
        let _ = cr.set_source(&g);
        let _ = cr.paint();
    });
    let c = clock.clone();
    bg.add_tick_callback(move |_, fc| {
        c.now.set(fc.frame_time() as f64 / 1e6);
        glib::ControlFlow::Continue
    });
    let back = gtk::Button::builder().label(t("back")).css_classes(["flat", "sv-tour-nav"]).build();
    let next = gtk::Button::builder().label(t("tw_start")).css_classes(["suggested-action", "pill", "sv-tour-next"]).build();
    let dots = adw::CarouselIndicatorLines::builder().carousel(&car).build();
    let foot = gtk::CenterBox::builder().margin_start(24).margin_end(24).margin_bottom(22).build();
    foot.set_start_widget(Some(&back));
    foot.set_center_widget(Some(&dots));
    foot.set_end_widget(Some(&next));
    let skip = gtk::Button::builder().label(t("skip")).css_classes(["flat", "sv-tour-nav"]).halign(gtk::Align::End).margin_top(14).margin_end(18).build();
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    col.append(&skip);
    col.append(&car);
    col.append(&foot);
    let over = gtk::Overlay::builder().child(&bg).build();
    over.add_overlay(&col);
    over.add_css_class("sv-tour");
    let d = adw::Dialog::builder().child(&over).content_width(980).content_height(680).build();

    // Page change: remember entry time, replay text animation, update buttons.
    let reveals = Rc::new(reveals);
    let on_page = {
        let (clock, reveals, next, back) = (clock.clone(), reveals.clone(), next.clone(), back.clone());
        move |i: u32| {
            clock.page.set(i);
            if let Some(e) = clock.entered.borrow_mut().get_mut(i as usize) {
                *e = if clock.now.get() > 0. { clock.now.get() } else { -1. };
            }
            for (k, w) in reveals.get(i as usize).into_iter().flatten().enumerate() {
                reveal(w, 60 + k as u32 * 110);
            }
            next.set_label(t(if i == 0 { "tw_start" } else if i == last { "td_go" } else { "next" }));
            back.set_opacity(if i == 0 { 0. } else { 1. });
            back.set_sensitive(i > 0);
        }
    };
    let op = on_page.clone();
    car.connect_page_changed(move |_, i| {
        op(i)
    });
    let (c2, d2) = (car.clone(), d.clone());
    let go = Rc::new(move |delta: i32| {
        let i = c2.position().round() as i32 + delta;
        if i >= c2.n_pages() as i32 {
            d2.close();
        } else if i >= 0 {
            c2.scroll_to(&c2.nth_page(i as u32), true);
        }
    });
    let g = go.clone();
    next.connect_clicked(move |_| g(1));
    let g = go.clone();
    back.connect_clicked(move |_| g(-1));
    let d2 = d.clone();
    skip.connect_clicked(move |_| {
        d2.close();
    });
    let keys = gtk::EventControllerKey::new();
    let g = go.clone();
    keys.connect_key_pressed(move |_, k, _, _| match k {
        gtk::gdk::Key::Right | gtk::gdk::Key::Return | gtk::gdk::Key::space => {
            g(1);
            glib::Propagation::Stop
        }
        gtk::gdk::Key::Left => {
            g(-1);
            glib::Propagation::Stop
        }
        _ => glib::Propagation::Proceed,
    });
    d.add_controller(keys);
    let c = ctx.clone();
    d.connect_closed(move |d| {
        // Closed for a language switch: the tour reopens itself, don't mark it finished.
        if unsafe { d.data::<bool>("sv-reopen") }.is_none() {
            let mut s = c.st.borrow_mut();
            s.settings.onboarded = true;
            s.settings.save();
        }
    });
    d.present(Some(&ctx.win));
    let (c2, op) = (car.clone(), on_page.clone());
    // After the dialog is laid out; scrolling earlier leaves the carousel between pages.
    glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
        if start > 0 {
            c2.scroll_to(&c2.nth_page(start), false);
        }
        op(start);
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn demo_skin_has_outer_layer() {
        let img = super::demo_skin(3);
        assert!(img.get_pixel(40, 0)[3] > 0 || img.get_pixel(44, 2)[3] > 0); // hat
        assert!(img.get_pixel(20, 40)[3] > 0); // jacket
        assert!(crate::tools::check(&img, false).iter().all(|i| !matches!(i, crate::tools::Issue::Unused(_))));
    }
}
