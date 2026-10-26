// Tool dialogs, selection/tab actions, crash recovery and the first-run tour.
use crate::i18n::t;
use crate::{skin, tools, AppCtx};
use adw::prelude::*;
use gtk::{gio, glib};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, rc::Rc};

const PART_GROUPS: [(&str, &[usize]); 5] = [("whole_skin", &[]), ("part_head", &[skin::HEAD]), ("part_body", &[skin::BODY]), ("arms", &[skin::RARM, skin::LARM]), ("legs", &[skin::RLEG, skin::LLEG])];

/// A dialog that previews its result on the live skin; Cancel restores, Apply records one undo step.
fn live_dialog(ctx: &Rc<AppCtx>, heading: &str, body: Option<&str>, extra: &impl IsA<gtk::Widget>) -> (adw::AlertDialog, Rc<dyn Fn(RgbaImage)>, RgbaImage) {
    let orig = ctx.st.borrow().skin.clone();
    let d = adw::AlertDialog::new(Some(heading), body);
    d.set_extra_child(Some(extra));
    d.add_responses(&[("cancel", t("cancel")), ("apply", t("apply"))]);
    d.set_response_appearance("apply", adw::ResponseAppearance::Suggested);
    d.set_close_response("cancel");
    let st = ctx.st.clone();
    let preview: Rc<dyn Fn(RgbaImage)> = Rc::new(move |img| {
        let mut s = st.borrow_mut();
        s.skin = img;
        s.refresh();
    });
    let (st, o) = (ctx.st.clone(), orig.clone());
    d.connect_response(None, move |_, r| {
        let mut s = st.borrow_mut();
        let new = std::mem::replace(&mut s.skin, o.clone());
        if r == "apply" && new != o {
            s.checkpoint();
            s.skin = new;
        }
        s.refresh();
    });
    (d, preview, orig)
}

fn scale(min: f64, max: f64, val: f64, label: &str, b: &gtk::Box) -> gtk::Scale {
    let s = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, (max - min) / 100.);
    s.set_value(val);
    s.set_draw_value(true);
    s.set_hexpand(true);
    b.append(&gtk::Label::builder().label(label).xalign(0.).build());
    b.append(&s);
    s
}

fn adjust(ctx: &Rc<AppCtx>) {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let names: Vec<&str> = PART_GROUPS.iter().map(|g| t(g.0)).collect();
    let target = gtk::DropDown::from_strings(&names);
    b.append(&target);
    let hue = scale(-180., 180., 0., t("hue"), &b);
    let sat = scale(0., 2., 1., t("saturation"), &b);
    let val = scale(0., 2., 1., t("brightness"), &b);
    let (d, preview, orig) = live_dialog(ctx, t("adjust_colors"), None, &b);
    let slim = ctx.st.borrow().slim;
    let upd = {
        let (target, hue, sat, val) = (target.clone(), hue.clone(), sat.clone(), val.clone());
        Rc::new(move || {
            let sel = PART_GROUPS[target.selected() as usize].1;
            preview(tools::adjust(&orig, slim, (!sel.is_empty()).then_some(sel), hue.value(), sat.value(), val.value()));
        })
    };
    for s in [&hue, &sat, &val] {
        let u = upd.clone();
        s.connect_value_changed(move |_| u());
    }
    target.connect_selected_notify(move |_| upd());
    d.present(Some(&ctx.win));
}

fn quantize(ctx: &Rc<AppCtx>) {
    let have = tools::count_colors(&ctx.st.borrow().skin);
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let n = gtk::SpinButton::with_range(2., 64., 1.);
    n.set_value(16f64.min(have as f64));
    b.append(&n);
    let (d, preview, orig) = live_dialog(ctx, t("reduce_colors"), Some(&format!("{} {have}", t("colors_now"))), &b);
    let upd = move |n: &gtk::SpinButton| preview(tools::quantize(&orig, n.value() as usize));
    upd(&n);
    n.connect_value_changed(upd);
    d.present(Some(&ctx.win));
}

fn transfer(ctx: &Rc<AppCtx>) {
    let c = ctx.clone();
    ctx.png_dialog().open(Some(&ctx.win), None::<&gio::Cancellable>, move |r| {
        let Some(p) = r.ok().and_then(|f| f.path()) else { return };
        let src = match std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| skin::load(&b).map_err(|d| format!("{} {d}", t("bad_size")))) {
            Ok(i) => i,
            Err(e) => return c.toast(&format!("{}: {e}", t("error"))),
        };
        let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let checks: Vec<(gtk::CheckButton, usize)> = ["part_head", "part_body", "part_rarm", "part_larm", "part_rleg", "part_lleg"]
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let cb = gtk::CheckButton::with_label(t(k));
                b.append(&cb);
                (cb, i)
            })
            .collect();
        let (d, preview, orig) = live_dialog(&c, t("transfer_parts"), Some(&p.file_name().unwrap_or_default().to_string_lossy()), &b);
        let slim = c.st.borrow().slim;
        let all: Vec<_> = checks.iter().map(|(cb, i)| (cb.clone(), *i)).collect();
        let upd = Rc::new(move || {
            let sel: Vec<usize> = all.iter().filter(|(cb, _)| cb.is_active()).map(|(_, i)| *i).collect();
            let mut img = orig.clone();
            tools::transfer(&mut img, &src, slim, &sel);
            preview(img);
        });
        for (cb, _) in &checks {
            let u = upd.clone();
            cb.connect_toggled(move |_| u());
        }
        d.present(Some(&c.win));
    });
}

fn check(ctx: &Rc<AppCtx>) {
    let (issues, slim) = {
        let s = ctx.st.borrow();
        (tools::check(&s.skin, s.slim), s.slim)
    };
    let body = if issues.is_empty() {
        t("check_ok").to_string()
    } else {
        issues
            .iter()
            .map(|i| match i {
                tools::Issue::TransparentBase(n) => format!("• {n} {}", t("issue_transparent")),
                tools::Issue::Unused(n) => format!("• {n} {}", t("issue_unused")),
                tools::Issue::SemiTransparent(n) => format!("• {n} {}", t("issue_semi")),
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let d = adw::AlertDialog::new(Some(t("check_skin")), Some(&body));
    d.add_response("close", t("close"));
    if !issues.is_empty() {
        d.add_response("fix", t("fix_all"));
        d.set_response_appearance("fix", adw::ResponseAppearance::Suggested);
    }
    let st = ctx.st.clone();
    d.connect_response(None, move |_, r| {
        if r == "fix" {
            let mut s = st.borrow_mut();
            s.checkpoint();
            tools::fix(&mut s.skin, slim);
            s.refresh();
        }
    });
    d.present(Some(&ctx.win));
}

fn close_tab(ctx: &Rc<AppCtx>, i: usize) {
    let ask = {
        let s = ctx.st.borrow();
        s.tab_dirty(i) && s.settings.confirm_unsaved
    };
    let st = ctx.st.clone();
    if !ask {
        return st.borrow_mut().close_tab(i);
    }
    let d = adw::AlertDialog::new(Some(t("unsaved")), Some(t("unsaved_body")));
    d.add_responses(&[("cancel", t("cancel")), ("discard", t("discard"))]);
    d.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
    d.connect_response(None, move |_, r| {
        if r == "discard" {
            st.borrow_mut().close_tab(i);
        }
    });
    d.present(Some(&ctx.win));
}

/// Cairo ARGB32 frame -> straight RGBA image.
pub fn frame_to_image(color: &[u8], w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| {
        let i = ((y * w + x) * 4) as usize;
        let a = color[i + 3];
        let un = |c: u8| if a == 0 { 0 } else { (c as u32 * 255 / a as u32).min(255) as u8 };
        image::Rgba([un(color[i + 2]), un(color[i + 1]), un(color[i]), a])
    })
}

fn save_dialog(ctx: &Rc<AppCtx>, name: &str, write: impl Fn(&std::path::Path) -> Result<(), String> + 'static) {
    let d = gtk::FileDialog::builder().modal(true).initial_name(name).build();
    let c = ctx.clone();
    d.save(Some(&ctx.win), None::<&gio::Cancellable>, move |r| {
        if let Some(p) = r.ok().and_then(|f| f.path()) {
            match write(&p) {
                Ok(()) => c.toast(&format!("{} {}", t("saved"), p.display())),
                Err(e) => c.toast(&format!("{}: {e}", t("error"))),
            }
        }
    });
}

/// 48-frame turntable GIF of the current 3D scene (animation included).
fn gif(ctx: &Rc<AppCtx>) {
    use image::codecs::gif::{GifEncoder, Repeat};
    let frames: Vec<RgbaImage> = {
        let s = ctx.st.borrow();
        let (yaw, t0) = s.anim_state();
        (0..48)
            .map(|i| {
                let fr = s.render_view(360, 360, yaw + i as f32 * std::f32::consts::TAU / 48., t0 + i as f32 * 0.26, None, false);
                frame_to_image(&fr.color, 360, 360)
            })
            .collect()
    };
    save_dialog(ctx, "skinvas.gif", move |p| {
        let f = std::fs::File::create(p).map_err(|e| e.to_string())?;
        let mut enc = GifEncoder::new_with_speed(f, 10);
        enc.set_repeat(Repeat::Infinite).map_err(|e| e.to_string())?;
        let delay = image::Delay::from_numer_denom_ms(40, 1);
        enc.encode_frames(frames.iter().map(|i| image::Frame::from_parts(i.clone(), 0, 0, delay))).map_err(|e| e.to_string())
    });
}

/// 1024x1024 PNG of the current view (transparent background in the studio scene).
fn frame(ctx: &Rc<AppCtx>) {
    let img = {
        let s = ctx.st.borrow();
        let (yaw, t0) = s.anim_state();
        frame_to_image(&s.render_view(1024, 1024, yaw, t0, None, true).color, 1024, 1024)
    };
    save_dialog(ctx, "skinvas-render.png", move |p| img.save_with_format(p, image::ImageFormat::Png).map_err(|e| e.to_string()));
}

/// Puts a picture onto the face, chest, back or the whole front, with live preview.
fn picture(ctx: &Rc<AppCtx>) {
    let f = gtk::FileFilter::new();
    f.set_name(Some(t("images")));
    f.add_mime_type("image/*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&f);
    let c = ctx.clone();
    gtk::FileDialog::builder().modal(true).filters(&filters).build().open(Some(&ctx.win), None::<&gio::Cancellable>, move |r| {
        let Some(p) = r.ok().and_then(|f| f.path()) else { return };
        let pic = match image::open(&p) {
            Ok(i) => i,
            Err(e) => return c.toast(&format!("{}: {e}", t("error"))),
        };
        let names: Vec<&str> = tools::PIC_TARGETS.iter().map(|k| t(k)).collect();
        let target = gtk::DropDown::from_strings(&names);
        let (d, preview, orig) = live_dialog(&c, t("from_picture"), Some(&p.file_name().unwrap_or_default().to_string_lossy()), &target);
        let slim = c.st.borrow().slim;
        let upd = move |dd: &gtk::DropDown| {
            let mut img = orig.clone();
            tools::picture(&mut img, &pic, slim, dd.selected() as usize);
            preview(img);
        };
        upd(&target);
        target.connect_selected_notify(upd);
        d.present(Some(&c.win));
    });
}

/// GIF of the whole painting process: layout on the left, model on the right.
fn timelapse(ctx: &Rc<AppCtx>) {
    use image::codecs::gif::{GifEncoder, Repeat};
    let (all, slim) = {
        let s = ctx.st.borrow();
        (s.timelapse(), s.slim)
    };
    if all.len() < 2 {
        return ctx.toast(t("timelapse_empty"));
    }
    // At most 120 frames, evenly spread, always ending on the final skin.
    let step = (all.len() as f64 / 120.).max(1.);
    let mut idx: Vec<usize> = (0..).map(|i| (i as f64 * step) as usize).take_while(|&i| i < all.len()).collect();
    if idx.last() != Some(&(all.len() - 1)) {
        idx.push(all.len() - 1);
    }
    let n = idx.len();
    let frames: Vec<RgbaImage> = idx
        .iter()
        .enumerate()
        .map(|(k, &i)| {
            let img = &all[i];
            let mut f = RgbaImage::from_pixel(512, 256, image::Rgba([30, 31, 36, 255]));
            let flat = image::imageops::resize(img, 224, 224, image::imageops::FilterType::Nearest);
            image::imageops::overlay(&mut f, &flat, 16, 16);
            let cam = crate::render::Camera { yaw: 0.4 + k as f32 / n as f32 * std::f32::consts::TAU, pitch: 0.2, dist: 62., fov: 35., target: [0., 16., 0.] };
            image::imageops::overlay(&mut f, &crate::packs::render_png(img, slim, 256, 256, cam), 256, 0);
            f
        })
        .collect();
    save_dialog(ctx, "skinvas-timelapse.gif", move |p| {
        let file = std::fs::File::create(p).map_err(|e| e.to_string())?;
        let mut enc = GifEncoder::new_with_speed(file, 10);
        enc.set_repeat(Repeat::Infinite).map_err(|e| e.to_string())?;
        let last = frames.len() - 1;
        enc.encode_frames(frames.iter().enumerate().map(|(i, f)| image::Frame::from_parts(f.clone(), 0, 0, image::Delay::from_numer_denom_ms(if i == last { 2000 } else { 70 }, 1))))
            .map_err(|e| e.to_string())
    });
}

/// QR code of a link, drawn as a white square with dark modules.
fn qr(link: &str) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::builder().content_width(200).content_height(200).halign(gtk::Align::Center).build();
    if let Ok(code) = qrcode::QrCode::new(link.as_bytes()) {
        let (w, cells) = (code.width(), code.to_colors());
        a.set_draw_func(move |_, cr, sw, _| {
            let m = sw as f64 / (w + 4) as f64;
            cr.set_source_rgb(1., 1., 1.);
            let _ = cr.paint();
            cr.set_source_rgb(0., 0., 0.);
            for (i, c) in cells.iter().enumerate() {
                if *c == qrcode::Color::Dark {
                    cr.rectangle(((i % w) + 2) as f64 * m, ((i / w) + 2) as f64 * m, m + 0.5, m + 0.5);
                }
            }
            let _ = cr.fill();
        });
    }
    a
}

fn mineskin(ctx: &Rc<AppCtx>) {
    let (png, slim, name) = {
        let s = ctx.st.borrow();
        let mut b = std::io::Cursor::new(Vec::new());
        let _ = s.skin.write_to(&mut b, image::ImageFormat::Png);
        (b.into_inner(), s.slim, s.title().trim_end_matches(" •").trim_end_matches(".png").to_string())
    };
    ctx.toast(t("uploading"));
    let c = ctx.clone();
    crate::browser::fetch(
        move || crate::net::mineskin_upload(&png, &name, slim),
        move |r| match r {
            Ok((link, tex)) => {
                c.st.borrow_mut().emote(2);
                let d = adw::AlertDialog::new(Some(t("mineskin_done")), Some(&format!("{link}\n\n{}\n{tex}", t("texture_url"))));
                d.set_extra_child(Some(&qr(&link)));
                d.add_responses(&[("close", t("close")), ("copy", t("copy_link"))]);
                d.set_response_appearance("copy", adw::ResponseAppearance::Suggested);
                let (c2, l2) = (c.clone(), link.clone());
                d.connect_response(None, move |_, r| {
                    if r == "copy" {
                        c2.win.clipboard().set_text(&l2);
                        c2.toast(t("copied"));
                    }
                });
                d.present(Some(&c.win));
            }
            Err(e) => c.toast(&format!("MineSkin: {e}")),
        },
    );
}

fn run_plugin(ctx: &Rc<AppCtx>, path: &str) {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return ctx.toast(&format!("{}: {e}", t("error"))),
    };
    let r = {
        let s = ctx.st.borrow();
        crate::plugins::run(&src, &s.skin, s.slim)
    };
    match r {
        Ok(img) => {
            let mut s = ctx.st.borrow_mut();
            if img != s.skin {
                s.checkpoint();
                s.skin = img;
                s.refresh();
            }
        }
        Err(e) => ctx.toast(&format!("{}: {e}", t("plugin_error"))),
    }
}

pub fn actions(app: &adw::Application, ctx: &Rc<AppCtx>) {
    let plain: [(&str, fn(&Rc<AppCtx>)); 16] = [
        ("picture", picture),
        ("timelapse", timelapse),
        ("mineskin", mineskin),
        ("plugins-folder", |c| {
            let _ = crate::plugins::list();
            gtk::FileLauncher::new(Some(&gio::File::for_path(crate::plugins::dir()))).launch(Some(&c.win), None::<&gio::Cancellable>, |_| {});
        }),
        ("gif", gif),
        ("frame", frame),
        ("adjust", adjust),
        ("quantize", quantize),
        ("transfer", transfer),
        ("check", check),
        ("random", |c| {
            let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64;
            c.st.borrow_mut().open_doc(tools::random_skin(seed), false, None, false);
            c.go("editor");
        }),
        ("sel-copy", |c| c.st.borrow_mut().sel_copy()),
        ("sel-paste", |c| c.st.borrow_mut().sel_paste()),
        ("sel-rotate", |c| c.st.borrow_mut().sel_transform(tools::rotate90)),
        ("sel-flip", |c| c.st.borrow_mut().sel_transform(tools::flip_h)),
        ("sel-delete", |c| c.st.borrow_mut().sel_delete()),
    ];
    for (name, f) in plain {
        let a = gio::SimpleAction::new(name, None);
        let c = ctx.clone();
        a.connect_activate(move |_, _| f(&c));
        app.add_action(&a);
    }
    let a = gio::SimpleAction::new("open-path", Some(glib::VariantTy::STRING));
    let c = ctx.clone();
    a.connect_activate(move |_, v| {
        if let Some(p) = v.and_then(|v| v.get::<String>()) {
            c.load_path(std::path::Path::new(&p));
        }
    });
    app.add_action(&a);
    let a = gio::SimpleAction::new("plugin", Some(glib::VariantTy::STRING));
    let c = ctx.clone();
    a.connect_activate(move |_, v| {
        if let Some(p) = v.and_then(|v| v.get::<String>()) {
            run_plugin(&c, &p);
        }
    });
    app.add_action(&a);
    let a = gio::SimpleAction::new("close-tab", Some(glib::VariantTy::UINT32));
    let c = ctx.clone();
    a.connect_activate(move |_, v| {
        if let Some(i) = v.and_then(|v| v.get::<u32>()) {
            close_tab(&c, i as usize);
        }
    });
    app.add_action(&a);
}

// --- crash recovery ---

#[derive(Serialize, Deserialize)]
struct Saved {
    id: u64,
    slim: bool,
    file: Option<PathBuf>,
}

fn rec_dir() -> PathBuf {
    crate::library::dir().join("recovery")
}

/// Writes every unsaved tab to the recovery folder.
pub fn autosave(ctx: &AppCtx) {
    let s = ctx.st.borrow();
    let docs = s.dirty_docs();
    let dir = rec_dir();
    if docs.is_empty() {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return eprintln!("skinvas: autosave: {e}");
    }
    let mut list = vec![];
    for (id, img, slim, file) in docs {
        if img.save_with_format(dir.join(format!("{id}.png")), image::ImageFormat::Png).is_ok() {
            list.push(Saved { id, slim, file: file.cloned() });
        }
    }
    let tmp = dir.join("index.json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec(&list).unwrap()).is_ok() {
        let _ = std::fs::rename(tmp, dir.join("index.json"));
    }
}

pub fn clear_recovery() {
    let _ = std::fs::remove_dir_all(rec_dir());
}

/// After a crash: offer to reopen the unsaved skins.
pub fn offer_recovery(ctx: &Rc<AppCtx>) {
    let dir = rec_dir();
    let old = crate::library::dir().join("recovery-pending");
    let _ = std::fs::remove_dir_all(&old);
    // Moved aside so the autosave timer can't overwrite it while the dialog is open.
    if std::fs::rename(&dir, &old).is_err() {
        return;
    }
    let list: Vec<Saved> = std::fs::read(old.join("index.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    if list.is_empty() {
        let _ = std::fs::remove_dir_all(&old);
        return;
    }
    let d = adw::AlertDialog::new(Some(t("recover_title")), Some(&format!("{} {}", t("recover_body"), list.len())));
    d.add_responses(&[("drop", t("discard")), ("restore", t("restore"))]);
    d.set_response_appearance("restore", adw::ResponseAppearance::Suggested);
    d.set_response_appearance("drop", adw::ResponseAppearance::Destructive);
    let c = ctx.clone();
    d.connect_response(None, move |_, r| {
        if r == "restore" {
            for e in &list {
                if let Some(img) = std::fs::read(old.join(format!("{}.png", e.id))).ok().and_then(|b| skin::load(&b).ok()) {
                    let mut s = c.st.borrow_mut();
                    s.open_doc(img, e.slim, e.file.clone(), false);
                    s.mark_dirty();
                }
            }
        }
        let _ = std::fs::remove_dir_all(&old);
    });
    d.present(Some(&ctx.win));
}
