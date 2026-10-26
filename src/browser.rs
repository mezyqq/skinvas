// Skin browser: online catalog (Ely.by, MineSkin, nicknames, URLs), local library, friends.
use crate::i18n::t;
use crate::library::{self, Friend, Library};
use crate::net::{self, Found, Source};
use crate::render::{self, Camera, ModelOpts};
use crate::{skin, AppCtx};
use adw::prelude::*;
use gtk::{cairo, gio, glib};
use image::RgbaImage;
use std::{cell::RefCell, rc::Rc};

/// Run `work` on a worker thread, then `done` on the GTK thread.
pub fn fetch<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    glib::spawn_future_local(async move {
        if let Ok(r) = gio::spawn_blocking(work).await {
            done(r);
        }
    });
}

pub fn thumb(img: &RgbaImage, slim: bool, voxels: bool, w: usize, h: usize) -> Option<cairo::ImageSurface> {
    let opts = ModelOpts { slim, base: true, overlay: true, voxels, thickness: 1. };
    let cam = Camera { yaw: 0.55, pitch: 0.2, dist: 62., fov: 35., target: [0., 16., 0.] };
    let mut fr = render::render(&render::build(img, &opts), img, slim, &[[0.; 3]; 6], &cam, w, h, [0, 0, 0, 0], true);
    cairo::ImageSurface::create_for_data(std::mem::take(&mut fr.color), cairo::Format::ARgb32, w as i32, h as i32, w as i32 * 4).ok()
}

type Loaded = Rc<RefCell<Option<(RgbaImage, bool, Vec<u8>)>>>;

struct Card {
    root: gtk::Box,
    area: gtk::DrawingArea,
    actions: gtk::Box,
    data: Loaded,
}

const TW: i32 = 150;
const TH: i32 = 190;

impl Card {
    fn new(ctx: &Rc<AppCtx>, title: &str, sub: Option<&str>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.add_css_class("sv-card");
        let area = gtk::DrawingArea::builder().content_width(TW).content_height(TH).halign(gtk::Align::Center).build();
        let data: Loaded = Rc::default();
        let surf: Rc<RefCell<Option<cairo::ImageSurface>>> = Rc::default();
        let (d, sf, st) = (data.clone(), surf.clone(), ctx.st.clone());
        area.set_draw_func(move |_, cr, w, h| {
            if sf.borrow().is_none() {
                if let Some((img, slim, _)) = &*d.borrow() {
                    *sf.borrow_mut() = thumb(img, *slim, st.borrow().settings.layers3d, w as usize, h as usize);
                }
            }
            match &*sf.borrow() {
                Some(s) => {
                    let _ = cr.set_source_surface(s, 0., 0.);
                    let _ = cr.paint();
                }
                None => {
                    cr.set_source_rgba(0.5, 0.5, 0.5, 0.15);
                    cr.rectangle(w as f64 * 0.3, h as f64 * 0.1, w as f64 * 0.4, h as f64 * 0.8);
                    let _ = cr.fill();
                }
            }
        });
        root.append(&area);
        let l = gtk::Label::builder().label(title).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(18).css_classes(["heading"]).build();
        l.set_tooltip_text(Some(title));
        root.append(&l);
        if let Some(sub) = sub {
            root.append(&gtk::Label::builder().label(sub).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(20).css_classes(["dim-label", "caption"]).build());
        }
        let actions = gtk::Box::builder().spacing(4).halign(gtk::Align::Center).build();
        root.append(&actions);
        Card { root, area, actions, data }
    }

    fn set(&self, img: RgbaImage, slim: bool, png: Vec<u8>) {
        *self.data.borrow_mut() = Some((img, slim, png));
        self.area.queue_draw();
    }

    fn load_url(&self, ctx: &Rc<AppCtx>, url: String, slim: Option<bool>) {
        let (data, area) = (self.data.clone(), self.area.clone());
        let _ = ctx;
        fetch(move || net::image(&url), move |r| match r.and_then(|b| skin::load(&b).map(|i| (i, b))) {
            Ok((img, b)) => {
                let slim = slim.unwrap_or_else(|| skin::detect_slim(&img));
                *data.borrow_mut() = Some((img, slim, b));
                area.queue_draw();
            }
            Err(e) => eprintln!("skinvas: {e}"),
        });
    }

    fn button(&self, label: &str, suggested: bool, f: impl Fn(&Loaded) + 'static) -> gtk::Button {
        let b = gtk::Button::with_label(label);
        if suggested {
            b.add_css_class("suggested-action");
        }
        let d = self.data.clone();
        b.connect_clicked(move |_| f(&d));
        self.actions.append(&b);
        b
    }

    fn open_button(&self, ctx: &Rc<AppCtx>, name: String) {
        let c = ctx.clone();
        self.button(t("open"), true, move |d| {
            if let Some((img, slim, _)) = d.borrow().clone() {
                c.open_skin(img, slim, &name);
            }
        });
    }
}

fn grid() -> gtk::FlowBox {
    gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).homogeneous(true).min_children_per_line(2).max_children_per_line(12).column_spacing(12).row_spacing(12).valign(gtk::Align::Start).margin_start(12).margin_end(12).margin_top(12).margin_bottom(12).build()
}

fn clear(fb: &gtk::FlowBox) {
    while let Some(c) = fb.first_child() {
        fb.remove(&c);
    }
}

fn liked(lib: &Library) -> String {
    lib.collections.get(1).or(lib.collections.first()).cloned().unwrap_or_default()
}

fn catalog(ctx: &Rc<AppCtx>) -> gtk::Widget {
    let sources = [t("src_ely"), t("src_mineskin"), t("src_mojang"), t("src_elynick"), t("src_local")];
    let src = gtk::DropDown::from_strings(&sources);
    let entry = gtk::SearchEntry::builder().placeholder_text(t("search_hint")).hexpand(true).build();
    let top = gtk::Box::builder().spacing(8).margin_start(12).margin_end(12).margin_top(12).build();
    top.append(&src);
    top.append(&entry);
    let fb = grid();
    let status = gtk::Label::builder().css_classes(["dim-label"]).margin_top(12).build();
    let more = gtk::Button::builder().label(t("more")).halign(gtk::Align::Center).margin_bottom(16).visible(false).build();
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    col.append(&status);
    col.append(&fb);
    col.append(&more);
    let scroll = gtk::ScrolledWindow::builder().child(&col).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();

    let cursor: Rc<RefCell<String>> = Rc::default();
    // Generation counter: results of an outdated search are dropped.
    let generation = Rc::new(std::cell::Cell::new(0u32));
    let run: Rc<dyn Fn(bool)> = {
        let (ctx, src, entry, fb, status, more, cursor, generation) = (ctx.clone(), src.clone(), entry.clone(), fb.clone(), status.clone(), more.clone(), cursor.clone(), generation.clone());
        Rc::new(move |append: bool| {
            let source = [Source::Ely, Source::MineSkin, Source::Mojang, Source::ElyNick, Source::Local][src.selected() as usize];
            let q = entry.text().to_string();
            if !append {
                clear(&fb);
                cursor.borrow_mut().clear();
                if source == Source::Ely {
                    *cursor.borrow_mut() = "1".into();
                }
            }
            generation.set(generation.get() + 1);
            let my = generation.get();
            status.set_label(t("loading"));
            status.set_visible(true);
            more.set_visible(false);
            let cur = cursor.borrow().clone();
            let (ctx, fb, status, more, cursor, generation) = (ctx.clone(), fb.clone(), status.clone(), more.clone(), cursor.clone(), generation.clone());
            let direct = q.trim().starts_with("http://") || q.trim().starts_with("https://");
            fetch(
                move || if direct { Ok((vec![Found { name: q.trim().rsplit('/').next().unwrap_or("skin").into(), url: q.trim().into(), slim: None }], None)) } else { net::search(source, &q, &cur) },
                move |r| {
                    if generation.get() != my {
                        return;
                    }
                    match r {
                        Ok((items, next)) => {
                            status.set_visible(items.is_empty() && !append);
                            status.set_label(t("nothing_found"));
                            for f in items {
                                let card = Card::new(&ctx, &f.name, None);
                                card.load_url(&ctx, f.url.clone(), f.slim);
                                card.open_button(&ctx, f.name.clone());
                                let c = ctx.clone();
                                let name = f.name.clone();
                                card.button("★", false, move |d| {
                                    if let Some((_, slim, png)) = d.borrow().clone() {
                                        let mut lib = c.lib.borrow_mut();
                                        let coll = liked(&lib);
                                        match lib.add(&png, &name, slim, &coll) {
                                            Ok(()) => c.toast(&format!("{} «{coll}»", t("added_to"))),
                                            Err(e) => c.toast(&format!("{}: {e}", t("error"))),
                                        }
                                        drop(lib);
                                        c.library_changed();
                                    }
                                })
                                .set_tooltip_text(Some(t("to_library")));
                                if matches!(source, Source::Mojang | Source::ElyNick) && !direct {
                                    let (c, nick, ely) = (ctx.clone(), f.name.clone(), source == Source::ElyNick);
                                    card.button("+", false, move |_| c.add_friend(&nick, ely)).set_tooltip_text(Some(t("add_friend")));
                                }
                                fb.append(&card.root);
                            }
                            more.set_visible(next.is_some());
                            *cursor.borrow_mut() = next.unwrap_or_default();
                        }
                        Err(e) => {
                            status.set_visible(true);
                            status.set_label(&format!("{}: {e}", t("error")));
                        }
                    }
                },
            );
        })
    };
    let r = run.clone();
    entry.connect_activate(move |_| r(false));
    let r = run.clone();
    src.connect_selected_notify(move |_| r(false));
    let r = run.clone();
    more.connect_clicked(move |_| r(true));
    // Load the popular list the first time the tab is shown.
    let (r, done) = (run.clone(), std::cell::Cell::new(false));
    scroll.connect_map(move |_| {
        if !done.replace(true) {
            r(false);
        }
    });
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&top);
    root.append(&scroll);
    root.upcast()
}

fn library_tab(ctx: &Rc<AppCtx>) -> gtk::Widget {
    let lib = ctx.lib.borrow();
    let mut names = vec![t("all").to_string()];
    names.extend(lib.collections.iter().cloned());
    drop(lib);
    let names_ref: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let coll = gtk::DropDown::from_strings(&names_ref);
    let entry = gtk::SearchEntry::builder().placeholder_text(t("lib_search")).hexpand(true).build();
    let add_cur = gtk::Button::with_label(t("save_current"));
    add_cur.add_css_class("suggested-action");
    let new_coll = gtk::Button::from_icon_name("folder-new-symbolic");
    new_coll.set_tooltip_text(Some(t("new_collection")));
    let top = gtk::Box::builder().spacing(8).margin_start(12).margin_end(12).margin_top(12).build();
    top.append(&coll);
    top.append(&entry);
    top.append(&new_coll);
    top.append(&add_cur);
    let fb = grid();
    let empty = adw::StatusPage::builder().icon_name("starred-symbolic").title(t("lib_empty")).description(t("lib_empty_body")).vexpand(true).build();
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    col.append(&empty);
    col.append(&fb);
    let scroll = gtk::ScrolledWindow::builder().child(&col).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();

    let fill: Rc<dyn Fn()> = {
        let (ctx, coll, entry, fb, empty) = (ctx.clone(), coll.clone(), entry.clone(), fb.clone(), empty.clone());
        Rc::new(move || {
            clear(&fb);
            let lib = ctx.lib.borrow();
            let filter = (coll.selected() > 0).then(|| lib.collections.get(coll.selected() as usize - 1).cloned()).flatten();
            let items: Vec<_> = lib.entries.iter().filter(|e| library::matches(e, filter.as_deref(), &entry.text())).cloned().collect();
            let collections = lib.collections.clone();
            drop(lib);
            empty.set_visible(items.is_empty());
            for e in items {
                let sub = if e.tags.is_empty() { e.collection.clone() } else { format!("{} · {}", e.collection, e.tags.join(", ")) };
                let card = Card::new(&ctx, &e.name, Some(&sub));
                if let Ok(b) = std::fs::read(Library::png_path(&e.id)) {
                    if let Ok(img) = skin::load(&b) {
                        card.set(img, e.slim, b);
                    }
                }
                card.open_button(&ctx, e.name.clone());
                let edit = gtk::MenuButton::builder().icon_name("document-edit-symbolic").tooltip_text(t("edit")).build();
                edit.set_popover(Some(&edit_popover(&ctx, &e, &collections)));
                card.actions.append(&edit);
                fb.append(&card.root);
            }
        })
    };
    ctx.lib_listeners.borrow_mut().push(fill.clone());
    let f = fill.clone();
    entry.connect_search_changed(move |_| f());
    let f = fill.clone();
    coll.connect_selected_notify(move |_| f());
    let (c, cl) = (ctx.clone(), coll.clone());
    add_cur.connect_clicked(move |_| {
        let (png, slim, name) = {
            let s = c.st.borrow();
            let mut b = std::io::Cursor::new(Vec::new());
            let _ = s.skin.write_to(&mut b, image::ImageFormat::Png);
            (b.into_inner(), s.slim, s.title().trim_end_matches(" •").to_string())
        };
        let mut lib = c.lib.borrow_mut();
        let target = (cl.selected() > 0).then(|| lib.collections.get(cl.selected() as usize - 1).cloned()).flatten().or(lib.collections.first().cloned()).unwrap_or_default();
        let r = lib.add(&png, &name, slim, &target);
        drop(lib);
        match r {
            Ok(()) => c.toast(&format!("{} «{target}»", t("added_to"))),
            Err(e) => c.toast(&format!("{}: {e}", t("error"))),
        }
        c.library_changed();
    });
    let c = ctx.clone();
    new_coll.connect_clicked(move |_| {
        let d = adw::AlertDialog::new(Some(t("new_collection")), None);
        let e = gtk::Entry::builder().activates_default(true).build();
        d.set_extra_child(Some(&e));
        d.add_responses(&[("cancel", t("cancel")), ("ok", t("create"))]);
        d.set_default_response(Some("ok"));
        d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        let c2 = c.clone();
        d.connect_response(None, move |_, r| {
            let name = e.text().trim().to_string();
            if r == "ok" && !name.is_empty() && !c2.lib.borrow().collections.contains(&name) {
                c2.lib.borrow_mut().collections.push(name);
                let _ = c2.lib.borrow().save();
                let c3 = c2.clone();
                // The collection dropdown is part of the page, so rebuild it.
                glib::idle_add_local_once(move || c3.rebuild());
            }
        });
        d.present(Some(&c.win));
    });
    fill();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&top);
    root.append(&scroll);
    root.upcast()
}

fn edit_popover(ctx: &Rc<AppCtx>, e: &library::Entry, collections: &[String]) -> gtk::Popover {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.set_margin_start(8);
    b.set_margin_end(8);
    b.set_margin_top(8);
    b.set_margin_bottom(8);
    let name = gtk::Entry::builder().text(&e.name).placeholder_text(t("pack_name")).build();
    let tags = gtk::Entry::builder().text(e.tags.join(", ")).placeholder_text(t("tags_hint")).build();
    let refs: Vec<&str> = collections.iter().map(|s| s.as_str()).collect();
    let coll = gtk::DropDown::from_strings(&refs);
    coll.set_selected(collections.iter().position(|c| *c == e.collection).unwrap_or(0) as u32);
    let save = gtk::Button::builder().label(t("save")).css_classes(["suggested-action"]).build();
    let del = gtk::Button::builder().label(t("delete")).css_classes(["destructive-action"]).build();
    b.append(&name);
    b.append(&tags);
    b.append(&coll);
    let row = gtk::Box::builder().spacing(8).homogeneous(true).build();
    row.append(&del);
    row.append(&save);
    b.append(&row);
    let pop = gtk::Popover::builder().child(&b).build();
    let (c, id, cols, p) = (ctx.clone(), e.id.clone(), collections.to_vec(), pop.clone());
    save.connect_clicked(move |_| {
        if let Some(en) = c.lib.borrow_mut().entries.iter_mut().find(|x| x.id == id) {
            en.name = name.text().trim().to_string();
            en.tags = library::parse_tags(&tags.text());
            en.collection = cols.get(coll.selected() as usize).cloned().unwrap_or_default();
        }
        if let Err(e) = c.lib.borrow().save() {
            c.toast(&format!("{}: {e}", t("error")));
        }
        p.popdown();
        let c2 = c.clone();
        glib::idle_add_local_once(move || c2.library_changed());
    });
    let (c, id, p) = (ctx.clone(), e.id.clone(), pop.clone());
    del.connect_clicked(move |_| {
        let r = c.lib.borrow_mut().remove(&id);
        if let Err(e) = r {
            c.toast(&format!("{}: {e}", t("error")));
        }
        p.popdown();
        let c2 = c.clone();
        glib::idle_add_local_once(move || c2.library_changed());
    });
    pop
}

fn friends_tab(ctx: &Rc<AppCtx>) -> gtk::Widget {
    let entry = gtk::Entry::builder().placeholder_text(t("nick")).hexpand(true).build();
    let src = gtk::DropDown::from_strings(&["Mojang", "Ely.by"]);
    let add = gtk::Button::with_label(t("add_friend"));
    add.add_css_class("suggested-action");
    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.set_tooltip_text(Some(t("refresh")));
    let top = gtk::Box::builder().spacing(8).margin_start(12).margin_end(12).margin_top(12).build();
    top.append(&entry);
    top.append(&src);
    top.append(&add);
    top.append(&refresh);
    let fb = grid();
    let empty = adw::StatusPage::builder().icon_name("system-users-symbolic").title(t("friends_empty")).description(t("friends_empty_body")).vexpand(true).build();
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    col.append(&empty);
    col.append(&fb);
    let scroll = gtk::ScrolledWindow::builder().child(&col).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();

    let fill: Rc<dyn Fn()> = {
        let (ctx, fb, empty) = (ctx.clone(), fb.clone(), empty.clone());
        Rc::new(move || {
            clear(&fb);
            let friends: Vec<Friend> = ctx.lib.borrow().friends.clone();
            empty.set_visible(friends.is_empty());
            for f in friends {
                let card = Card::new(&ctx, &f.nick, Some(if f.ely { "Ely.by" } else { "Mojang" }));
                card.open_button(&ctx, f.nick.clone());
                let hist = gtk::MenuButton::builder().label(format!("{} ({})", t("history"), f.history.len())).build();
                hist.set_popover(Some(&history_popover(&ctx, &f)));
                card.actions.append(&hist);
                let (c, nick) = (ctx.clone(), f.nick.clone());
                card.button("✕", false, move |_| {
                    c.lib.borrow_mut().friends.retain(|x| x.nick != nick);
                    let _ = c.lib.borrow().save();
                    let c2 = c.clone();
                    glib::idle_add_local_once(move || c2.friends_changed());
                })
                .set_tooltip_text(Some(t("delete")));
                // Fetch the current skin; remember it in the local history if it changed.
                let (data, area, c, nick, ely) = (card.data.clone(), card.area.clone(), ctx.clone(), f.nick.clone(), f.ely);
                fetch(
                    move || {
                        let found = net::nick(if ely { Source::ElyNick } else { Source::Mojang }, &nick)?.ok_or("no skin")?;
                        let b = net::image(&found.url)?;
                        Ok::<_, String>((found, b))
                    },
                    move |r| {
                        if let Ok((found, b)) = r {
                            if let Ok(img) = skin::load(&b) {
                                let slim = found.slim.unwrap_or_else(|| skin::detect_slim(&img));
                                *data.borrow_mut() = Some((img, slim, b));
                                area.queue_draw();
                                let new = c.lib.borrow_mut().saw(&found.name, &found.url);
                                if new {
                                    let _ = c.lib.borrow().save();
                                }
                            }
                        }
                    },
                );
                fb.append(&card.root);
            }
        })
    };
    ctx.friend_listeners.borrow_mut().push(fill.clone());
    let (c, e, s) = (ctx.clone(), entry.clone(), src.clone());
    let do_add = move || {
        let nick = e.text().trim().to_string();
        if !nick.is_empty() {
            c.add_friend(&nick, s.selected() == 1);
            e.set_text("");
        }
    };
    let d = do_add.clone();
    entry.connect_activate(move |_| d());
    add.connect_clicked(move |_| do_add());
    let f = fill.clone();
    refresh.connect_clicked(move |_| f());
    fill();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&top);
    root.append(&scroll);
    root.upcast()
}

fn history_popover(ctx: &Rc<AppCtx>, f: &Friend) -> gtk::Popover {
    let fb = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(4).min_children_per_line(1).build();
    if f.history.is_empty() {
        fb.append(&gtk::Label::new(Some(t("history_empty"))));
    }
    for url in &f.history {
        let card = Card::new(ctx, "", None);
        card.load_url(ctx, url.clone(), None);
        card.open_button(ctx, f.nick.clone());
        fb.append(&card.root);
    }
    let scroll = gtk::ScrolledWindow::builder().child(&fb).min_content_width(360).min_content_height(300).build();
    gtk::Popover::builder().child(&scroll).build()
}

pub fn page(ctx: &Rc<AppCtx>) -> gtk::Widget {
    let vs = adw::ViewStack::new();
    vs.add_titled_with_icon(&catalog(ctx), Some("catalog"), t("catalog"), "web-browser-symbolic");
    vs.add_titled_with_icon(&library_tab(ctx), Some("library"), t("library"), "starred-symbolic");
    vs.add_titled_with_icon(&friends_tab(ctx), Some("friends"), t("friends"), "system-users-symbolic");
    let sw = adw::ViewSwitcher::builder().stack(&vs).policy(adw::ViewSwitcherPolicy::Wide).halign(gtk::Align::Center).margin_top(8).build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&sw);
    root.append(&vs);
    root.upcast()
}
