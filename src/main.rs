// No console window behind the app on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
mod browser;
mod colorpick;
mod dialogs;
mod editor;
mod i18n;
mod library;
mod net;
mod packs;
mod plugins;
mod discord;
mod render;
mod settings;
mod settings_page;
mod skin;
mod tools;
mod tour;

use adw::prelude::*;
use editor::{St, State};
use gtk::{gdk, gio, glib};
use i18n::t;
use settings::{parse_hex, Settings};
use std::{cell::RefCell, path::Path, rc::Rc};

pub struct AppCtx {
    pub st: St,
    win: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    css: gtk::CssProvider,
    page: RefCell<String>,
    nav: RefCell<Vec<(&'static str, gtk::ToggleButton)>>,
    base_dpi: i32,
    pub lib: RefCell<library::Library>,
    pub lib_listeners: RefCell<Vec<Rc<dyn Fn()>>>,
    pub friend_listeners: RefCell<Vec<Rc<dyn Fn()>>>,
    autosave_ticks: std::cell::Cell<u32>,
    discord: std::sync::mpsc::Sender<Option<discord::Presence>>,
    last_presence: RefCell<Option<discord::Presence>>,
}

impl AppCtx {
    pub fn toast(&self, msg: &str) {
        self.toasts.add_toast(adw::Toast::new(msg));
    }

    pub fn go(&self, page: &str) {
        if let Some((_, b)) = self.nav.borrow().iter().find(|(n, _)| *n == page) {
            b.set_active(true);
        }
    }

    /// Opens a skin from the browser/library/clipboard in the editor.
    pub fn open_skin(self: &Rc<Self>, img: image::RgbaImage, slim: bool, name: &str) {
        self.st.borrow_mut().open_doc(img, slim, None, false);
        self.go("editor");
        self.toast(&format!("{} {name}", t("opened")));
    }

    /// Pushes "what I'm doing" to Discord when it changes (only if enabled with an app ID).
    pub fn update_presence(&self) {
        let p = {
            let s = self.st.borrow();
            let id = s.settings.discord_id.trim().to_string();
            (s.settings.discord && !id.is_empty()).then(|| {
                let page = self.page.borrow().clone();
                let details = t(match page.as_str() {
                    "browser" => "rpc_browsing",
                    "packs" => "rpc_packs",
                    _ => "rpc_editing",
                });
                discord::Presence { client_id: id, details: details.into(), state: s.title().trim_end_matches(" •").to_string() }
            })
        };
        if *self.last_presence.borrow() != p {
            *self.last_presence.borrow_mut() = p.clone();
            let _ = self.discord.send(p);
        }
    }

    pub fn library_changed(&self) {
        let ls: Vec<_> = self.lib_listeners.borrow().clone();
        ls.iter().for_each(|f| f());
    }

    pub fn friends_changed(&self) {
        let ls: Vec<_> = self.friend_listeners.borrow().clone();
        ls.iter().for_each(|f| f());
    }

    pub fn add_friend(&self, nick: &str, ely: bool) {
        let mut lib = self.lib.borrow_mut();
        if lib.friends.iter().any(|f| f.nick.eq_ignore_ascii_case(nick) && f.ely == ely) {
            return;
        }
        lib.friends.push(library::Friend { nick: nick.into(), ely, history: vec![] });
        let r = lib.save();
        drop(lib);
        match r {
            Ok(()) => self.toast(&format!("{} {nick}", t("friend_added"))),
            Err(e) => self.toast(&format!("{}: {e}", t("error"))),
        }
        self.friends_changed();
    }

    /// Ctrl+V: an image, a link to a PNG, or a nickname from the clipboard.
    fn paste(self: &Rc<Self>) {
        let cb = self.win.clipboard();
        let c = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(Some(tex)) = cb.read_texture_future().await {
                match skin::load(&tex.save_to_png_bytes()) {
                    Ok(img) => c.open_skin(img.clone(), skin::detect_slim(&img), t("clipboard")),
                    Err(d) => c.toast(&format!("{} {d}", t("bad_size"))),
                }
                return;
            }
            let Ok(Some(text)) = cb.read_text_future().await else { return };
            let text = text.trim().to_string();
            let c2 = c.clone();
            browser::fetch(
                move || {
                    let url = if text.starts_with("http") { text.clone() } else { net::nick(net::Source::Mojang, &text)?.ok_or(t("nothing_found"))?.url };
                    skin::load(&net::image(&url)?).map(|i| (i, text)).map_err(|d| format!("{} {d}", t("bad_size")))
                },
                move |r| match r {
                    Ok((img, name)) => c2.open_skin(img.clone(), skin::detect_slim(&img), &name),
                    Err(e) => c2.toast(&format!("{}: {e}", t("error"))),
                },
            );
        });
    }

    pub fn apply_look(&self) {
        let s = &self.st.borrow().settings;
        let [r, g, b, _] = parse_hex(&s.accent);
        let fg = if 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32 > 160. { "#000000" } else { "#ffffff" };
        let rad = s.radius;
        let mut css = format!(
            ":root {{ --accent-bg-color: {a}; --accent-color: {a}; --accent-fg-color: {fg}; }}
            button, entry, dropdown > button, .card, popover > contents {{ border-radius: {rad}px; }}
            .sv-nav {{ border-radius: {rad}px; }}
            {BASE_CSS}",
            a = &s.accent[..7.min(s.accent.len())]
        );
        for (i, c) in settings_page::ACCENTS.iter().enumerate() {
            css += &format!(".sv-sw-{i} {{ background: {c}; }}\n");
        }
        // Explicit palettes beat a user gtk.css (e.g. caelestia) that forces its own colours.
        css += match s.theme.as_str() {
            "light" => &LIGHT_CSS,
            "dark" => &DARK_CSS,
            "amoled" => &AMOLED_CSS,
            _ => "",
        };
        net::CACHE_ON.store(s.cache, std::sync::atomic::Ordering::Relaxed);
        net::SAFE.store(s.safe, std::sync::atomic::Ordering::Relaxed);
        if s.tablet {
            css += TABLET_CSS;
        }
        self.css.load_from_string(&css);
        adw::StyleManager::default().set_color_scheme(match s.theme.as_str() {
            "light" => adw::ColorScheme::ForceLight,
            "system" => adw::ColorScheme::Default,
            _ => adw::ColorScheme::ForceDark,
        });
        if let Some(gs) = gtk::Settings::default() {
            gs.set_gtk_enable_animations(s.animations);
            gs.set_gtk_xft_dpi((self.base_dpi as f64 * s.font_scale) as i32);
        }
    }

    /// Re-creates the window content (used after a language change).
    pub fn rebuild(self: &Rc<Self>) {
        self.st.borrow_mut().listeners.clear();
        self.nav.borrow_mut().clear();
        self.lib_listeners.borrow_mut().clear();
        self.friend_listeners.borrow_mut().clear();
        self.toasts.set_child(Some(&content(self)));
    }

    fn load_path(&self, path: &Path) {
        match std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| skin::load(&b).map_err(|d| format!("{} {d}", t("bad_size")))) {
            Ok(img) => {
                let slim = skin::detect_slim(&img);
                self.st.borrow_mut().open_doc(img, slim, Some(path.to_path_buf()), false);
                self.remember(path);
                self.go("editor");
            }
            Err(e) => self.toast(&format!("{}: {e}", t("error"))),
        }
    }

    fn save_to(&self, path: &Path) {
        let r = self.st.borrow().skin.save_with_format(path, image::ImageFormat::Png);
        match r {
            Ok(()) => {
                let mut s = self.st.borrow_mut();
                s.mark_saved(path.to_path_buf());
                s.emote(0);
                drop(s);
                self.remember(path);
                self.toast(t("saved"));
            }
            Err(e) => self.toast(&format!("{}: {e}", t("error"))),
        }
    }

    /// Runs `then` right away, or after the user agrees to drop unsaved changes.
    /// Recent files list (newest first, max 10).
    fn remember(&self, path: &Path) {
        let p = path.to_string_lossy().into_owned();
        let mut s = self.st.borrow_mut();
        s.settings.recent.retain(|x| *x != p);
        s.settings.recent.insert(0, p);
        s.settings.recent.truncate(10);
        s.settings.save();
    }

    fn confirm_discard(self: &Rc<Self>, then: impl FnOnce() + 'static) {
        let ask = {
            let s = self.st.borrow();
            s.any_dirty() && s.settings.confirm_unsaved
        };
        if !ask {
            return then();
        }
        let d = adw::AlertDialog::new(Some(t("unsaved")), Some(t("unsaved_body")));
        d.add_responses(&[("cancel", t("cancel")), ("discard", t("discard"))]);
        d.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        d.set_default_response(Some("cancel"));
        let then = std::cell::Cell::new(Some(then));
        d.connect_response(None, move |_, r| {
            if r == "discard" {
                if let Some(f) = then.take() {
                    f();
                }
            }
        });
        d.present(Some(&self.win));
    }

    fn png_dialog(&self) -> gtk::FileDialog {
        let f = gtk::FileFilter::new();
        f.set_name(Some(t("png")));
        f.add_mime_type("image/png");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&f);
        gtk::FileDialog::builder().modal(true).filters(&filters).build()
    }
}

const BASE_CSS: &str = "
.sv-sidebar { padding: 14px 10px; min-width: 170px; background: alpha(currentColor, 0.035); border-right: 1px solid alpha(currentColor, 0.08); }
.sv-logo { font-size: 24px; font-weight: 800; margin: 6px 10px 20px 10px; }
.sv-logo-accent { color: var(--accent-bg-color); }
.sv-nav { padding: 10px 14px; background: transparent; font-weight: 600; box-shadow: none; }
.sv-nav:hover { background: alpha(currentColor, 0.07); }
.sv-nav:checked { background: var(--accent-bg-color); color: var(--accent-fg-color); }
.sv-toolbar { padding: 8px 10px; border-bottom: 1px solid alpha(currentColor, 0.08); }

.sv-floatbar { padding: 6px; border-radius: 99px; background: alpha(black, 0.55); color: white; }
.sv-floatbar button { border-radius: 99px; }
.sv-tabs { padding: 4px 8px; border-bottom: 1px solid alpha(currentColor, 0.08); }
.sv-tab { border-radius: 10px; padding-right: 2px; }
.sv-tab.active { background: alpha(var(--accent-bg-color), 0.25); }
.sv-status { padding: 4px 12px; border-top: 1px solid alpha(currentColor, 0.08); min-height: 22px; }
.sv-tour { color: white; }
.sv-tour-logo { font-size: 56px; font-weight: 900; letter-spacing: -1px; }
.sv-tour-title { font-size: 32px; font-weight: 800; }
.sv-tour-body { font-size: 16px; color: alpha(white, 0.78); }
.sv-tour-hello { font-size: 22px; font-weight: 800; }
.sv-tour-card { padding: 16px; border-radius: 20px; background: alpha(white, 0.07); color: white; box-shadow: none; transition: all 200ms ease; }
.sv-tour-card:hover { background: alpha(white, 0.14); }
.sv-tour-card.selected { box-shadow: inset 0 0 0 3px var(--accent-bg-color); background: alpha(white, 0.12); }
.sv-tour-panel { padding: 22px; border-radius: 22px; background: alpha(white, 0.07); }
.sv-tour-panel entry, .sv-tour-panel password { background: alpha(white, 0.1); color: white; }
.sv-tour-nav { color: alpha(white, 0.8); }
.sv-tour-next { padding: 10px 34px; font-weight: 800; font-size: 15px; }
.sv-tour carouselindicatorlines { color: white; }
.sv-card { padding: 10px; border-radius: 16px; background: alpha(currentColor, 0.05); }
.sv-card:hover { background: alpha(currentColor, 0.09); }
.sv-floatbar button:checked { background: var(--accent-bg-color); color: var(--accent-fg-color); }
button.sv-sw { min-width: 24px; min-height: 24px; padding: 0; border-radius: 99px; box-shadow: inset 0 0 0 1px alpha(black, 0.25); }
";

const TABLET_CSS: &str = "
button, togglebutton, menubutton > button, dropdown > button, spinbutton { min-height: 44px; min-width: 44px; }
.sv-nav { padding: 16px 18px; }
";

fn palette_css(bg: &str, fg: &str, view: &str, head: &str, card: &str, side: &str) -> String {
    let mut c = format!(
        ":root {{ --window-bg-color: {bg}; --window-fg-color: {fg}; --view-bg-color: {view}; --view-fg-color: {fg};
          --headerbar-bg-color: {head}; --headerbar-fg-color: {fg}; --sidebar-bg-color: {side}; --sidebar-fg-color: {fg};
          --card-bg-color: {card}; --card-fg-color: {fg}; --popover-bg-color: {card}; --popover-fg-color: {fg};
          --dialog-bg-color: {card}; --dialog-fg-color: {fg}; }}
        .sv-sidebar {{ background: {side}; color: {fg}; }}
        window, .background {{ background-color: {bg}; color: {fg}; }}
        headerbar {{ background-color: {head}; color: {fg}; }}
        popover > contents, popover > arrow {{ background-color: {card}; color: {fg}; }}\n"
    );
    for (n, v) in [("window_bg_color", bg), ("window_fg_color", fg), ("view_bg_color", view), ("view_fg_color", fg), ("headerbar_bg_color", head), ("headerbar_fg_color", fg), ("card_bg_color", card), ("card_fg_color", fg), ("popover_bg_color", card), ("popover_fg_color", fg), ("sidebar_bg_color", side), ("sidebar_fg_color", fg)] {
        c += &format!("@define-color {n} {v};\n");
    }
    c
}

static LIGHT_CSS: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| palette_css("#f6f6f8", "#1d1d22", "#ffffff", "#ebebef", "#ffffff", "#ececf0"));
static DARK_CSS: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| palette_css("#1f1f24", "#f1f1f4", "#18181c", "#26262c", "#2b2b31", "#19191d"));
static AMOLED_CSS: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| palette_css("#000000", "#f1f1f4", "#000000", "#000000", "#0d0d0f", "#000000"));

fn content(ctx: &Rc<AppCtx>) -> gtk::Widget {
    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).hexpand(true).vexpand(true).build();
    stack.add_named(&editor::page(&ctx.st), Some("editor"));
    stack.add_named(&browser::page(ctx), Some("browser"));
    stack.add_named(&packs::page(ctx), Some("packs"));
    stack.add_named(&settings_page::page(ctx), Some("settings"));

    let side = gtk::Box::new(gtk::Orientation::Vertical, 4);
    side.add_css_class("sv-sidebar");
    let logo = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    logo.add_css_class("sv-logo");
    logo.append(&gtk::Label::new(Some("Skin")));
    let l2 = gtk::Label::new(Some("vas"));
    l2.add_css_class("sv-logo-accent");
    logo.append(&l2);
    side.append(&logo);

    let title = adw::WindowTitle::new("Skinvas", &ctx.st.borrow().title());
    let current = ctx.page.borrow().clone();
    let mut group: Option<gtk::ToggleButton> = None;
    for (name, icon) in [("editor", "document-edit-symbolic"), ("browser", "web-browser-symbolic"), ("packs", "package-x-generic-symbolic"), ("settings", "emblem-system-symbolic")] {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.append(&gtk::Image::from_icon_name(icon));
        row.append(&gtk::Label::new(Some(t(name))));
        let b = gtk::ToggleButton::builder().child(&row).active(name == current).build();
        b.add_css_class("sv-nav");
        ctx.nav.borrow_mut().push((name, b.clone()));
        b.set_group(group.as_ref());
        group.get_or_insert(b.clone());
        let (st2, c2) = (stack.clone(), ctx.clone());
        b.connect_toggled(move |b| {
            if b.is_active() {
                st2.set_visible_child_name(name);
                *c2.page.borrow_mut() = name.into();
                let c3 = c2.clone();
                glib::idle_add_local_once(move || c3.update_presence());
                let mut s = c2.st.borrow_mut();
                s.settings.last_page = name.into();
                s.settings.save();
            }
        });
        if name == "settings" {
            let spacer = gtk::Box::builder().vexpand(true).build();
            side.append(&spacer);
        }
        side.append(&b);
    }
    stack.set_visible_child_name(&current);

    let (t2, c2) = (title.clone(), Rc::downgrade(ctx));
    ctx.st.borrow_mut().listeners.push(Box::new(move |s| {
        t2.set_subtitle(&s.title());
        // Deferred: update_presence borrows the state, which is borrowed right now.
        if let Some(c) = c2.upgrade() {
            glib::idle_add_local_once(move || c.update_presence());
        }
    }));

    let header = adw::HeaderBar::builder().title_widget(&title).build();
    // One-click light/dark switch.
    let light = ctx.st.borrow().settings.theme == "light";
    let theme_btn = gtk::Button::from_icon_name(if light { "weather-clear-night-symbolic" } else { "weather-clear-symbolic" });
    theme_btn.set_tooltip_text(Some(t(if light { "to_dark" } else { "to_light" })));
    let c = ctx.clone();
    theme_btn.connect_clicked(move |b| {
        let to_light = c.st.borrow().settings.theme != "light";
        settings_page::commit(&c, |s| s.theme = if to_light { "light" } else { "dark" }.into(), false);
        b.set_icon_name(if to_light { "weather-clear-night-symbolic" } else { "weather-clear-symbolic" });
        b.set_tooltip_text(Some(t(if to_light { "to_dark" } else { "to_light" })));
        let c2 = c.clone();
        // The theme combo on the settings page shows the old value; rebuild it.
        glib::idle_add_local_once(move || c2.rebuild());
    });
    header.pack_end(&theme_btn);
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(&stack));
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    root.append(&side);
    root.append(&tv);
    root.upcast()
}

fn actions(app: &adw::Application, ctx: &Rc<AppCtx>) {
    let add = |name: &str, accels: &[&str], f: Box<dyn Fn(&Rc<AppCtx>)>| {
        let a = gio::SimpleAction::new(name, None);
        let c = ctx.clone();
        a.connect_activate(move |_, _| f(&c));
        app.add_action(&a);
        app.set_accels_for_action(&format!("app.{name}"), accels);
    };
    add("new", &["<Ctrl>n", "<Ctrl>t"], Box::new(|c| {
        let slim = c.st.borrow().settings.slim_default;
        c.st.borrow_mut().open_doc(skin::starter(), slim, None, true);
        c.go("editor");
    }));
    add("open", &["<Ctrl>o"], Box::new(|c| {
        let c2 = c.clone();
        c.png_dialog().open(Some(&c.win), None::<&gio::Cancellable>, move |r| {
            if let Some(p) = r.ok().and_then(|f| f.path()) {
                c2.load_path(&p);
            }
        });
    }));
    let save_as = |c: &Rc<AppCtx>| {
        let d = c.png_dialog();
        d.set_initial_name(Some("skin.png"));
        let c2 = c.clone();
        d.save(Some(&c.win), None::<&gio::Cancellable>, move |r| {
            if let Some(mut p) = r.ok().and_then(|f| f.path()) {
                if p.extension().is_none() {
                    p.set_extension("png");
                }
                c2.save_to(&p);
            }
        });
    };
    add("save", &["<Ctrl>s"], Box::new(move |c| {
        let file = c.st.borrow().file.clone();
        match file {
            Some(p) => c.save_to(&p),
            None => save_as(c),
        }
    }));
    add("save-as", &["<Ctrl><Shift>s"], Box::new(save_as));
    add("undo", &["<Ctrl>z"], Box::new(|c| c.st.borrow_mut().undo()));
    add("redo", &["<Ctrl><Shift>z", "<Ctrl>y"], Box::new(|c| c.st.borrow_mut().redo()));
    // No accel: Ctrl+V is handled in the bubble phase below so text fields keep their own paste.
    add("paste", &[], Box::new(|c| c.paste()));
}

fn build(app: &adw::Application) -> Rc<AppCtx> {
    let settings = Settings::load();
    let lib_settings_page = settings.last_page.clone();
    i18n::set_lang(&settings.lang);
    let display = gdk::Display::default().unwrap();
    let css = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_USER + 1);
    // The UI is designed around Adwaita symbolic icons; other themes lack some of them.
    if let Some(gs) = gtk::Settings::default() {
        gs.set_gtk_icon_theme_name(Some("Adwaita"));
    }
    // Our own app icon works even before it is installed system-wide.
    gtk::IconTheme::for_display(&display).add_search_path(concat!(env!("CARGO_MANIFEST_DIR"), "/data/icons"));
    gtk::Window::set_default_icon_name("io.github.skinvas");
    let base_dpi = gtk::Settings::default().map(|s| s.gtk_xft_dpi()).filter(|d| *d > 0).unwrap_or(96 * 1024);
    let win = adw::ApplicationWindow::builder().application(app).title("Skinvas").default_width(1320).default_height(820).build();
    let toasts = adw::ToastOverlay::new();
    win.set_content(Some(&toasts));
    let collections = [t("coll_mine"), t("coll_liked"), t("coll_friends")];
    let lib = RefCell::new(library::Library::load(&collections));
    let ctx = Rc::new(AppCtx {
        st: Rc::new(RefCell::new(State::new(settings))),
        win: win.clone(),
        toasts,
        css,
        page: RefCell::new(settings_page_name(&lib_settings_page)),
        nav: RefCell::default(),
        base_dpi,
        lib,
        lib_listeners: RefCell::default(),
        friend_listeners: RefCell::default(),
        autosave_ticks: Default::default(),
        discord: discord::start(),
        last_presence: RefCell::default(),
    });
    ctx.apply_look();
    ctx.rebuild();
    actions(app, &ctx);
    dialogs::actions(app, &ctx);

    // Drag & drop a PNG anywhere on the window.
    let drop = gtk::DropTarget::new(gio::File::static_type(), gdk::DragAction::COPY);
    let c = ctx.clone();
    drop.connect_drop(move |_, v, _, _| {
        let Some(p) = v.get::<gio::File>().ok().and_then(|f| f.path()) else { return false };
        c.load_path(&p);
        true
    });
    win.add_controller(drop);

    let keys = gtk::EventControllerKey::builder().propagation_phase(gtk::PropagationPhase::Bubble).build();
    let c = ctx.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let selecting = c.st.borrow().tool == editor::Tool::Select;
        match key {
            gdk::Key::v | gdk::Key::V if ctrl => {
                // Inside the selection tool Ctrl+V pastes the copied piece, otherwise it imports from the clipboard.
                if selecting && c.st.borrow().has_clip() { c.st.borrow_mut().sel_paste() } else { c.paste() }
                return glib::Propagation::Stop;
            }
            gdk::Key::c | gdk::Key::C if ctrl && selecting => c.st.borrow_mut().sel_copy(),
            gdk::Key::Delete if selecting => c.st.borrow_mut().sel_delete(),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    win.add_controller(keys);

    let (c, force) = (ctx.clone(), Rc::new(std::cell::Cell::new(false)));
    win.connect_close_request(move |w| {
        let ask = {
            let s = c.st.borrow();
            s.any_dirty() && s.settings.confirm_unsaved
        };
        if !ask || force.get() {
            // A clean exit: nothing to recover next time.
            dialogs::clear_recovery();
            return glib::Propagation::Proceed;
        }
        let (w2, f2) = (w.clone(), force.clone());
        c.confirm_discard(move || {
            f2.set(true);
            w2.close();
        });
        glib::Propagation::Stop
    });
    win.present();
    dialogs::offer_recovery(&ctx);
    // First run: fetch Steve from Mojang and swap him into the untouched starter tab.
    if !skin::steve_path().exists() {
        let c = ctx.clone();
        browser::fetch(
            || net::image(skin::STEVE_URL).and_then(|b| skin::load(&b).map(|i| (i, b)).map_err(|e| e.to_string())),
            move |r| {
                if let Ok((img, png)) = r {
                    let _ = std::fs::create_dir_all(library::dir()).and_then(|_| std::fs::write(skin::steve_path(), png));
                    c.st.borrow_mut().replace_pristine(img);
                }
            },
        );
    }
    let c = ctx.clone();
    glib::timeout_add_seconds_local(15, move || {
        // Ticks every 15 s; the setting decides how often we actually write.
        let every = c.st.borrow().settings.autosave_secs;
        let n = c.autosave_ticks.get() + 15;
        if every > 0 && n >= every {
            dialogs::autosave(&c);
            c.autosave_ticks.set(0);
        } else {
            c.autosave_ticks.set(n);
        }
        glib::ControlFlow::Continue
    });
    // Dev aid: SKINVAS_TOUR=<page> opens the welcome tour on that page (useful with SKINVAS_SHOT).
    if let Some(p) = std::env::var("SKINVAS_TOUR").ok().and_then(|p| p.parse().ok()) {
        tour::show_at(&ctx, p);
    } else if !ctx.st.borrow().settings.onboarded && std::env::var("SKINVAS_SHOT").is_err() {
        tour::show(&ctx);
    }
    // Dev aid: SKINVAS_SHOT=out.png renders the window to a PNG after a few seconds, then quits.
    if let Ok(path) = std::env::var("SKINVAS_SHOT") {
        let (w, app) = (win.clone(), app.clone());
        glib::timeout_add_seconds_local_once(7, move || {
            let p = gtk::WidgetPaintable::new(Some(&w));
            let snap = gtk::Snapshot::new();
            p.snapshot(&snap, w.width() as f64, w.height() as f64);
            if let (Some(node), Some(native)) = (snap.to_node(), w.native()) {
                let _ = native.renderer().unwrap().render_texture(node, None).save_to_png(&path);
            }
            app.quit();
        });
    }
    ctx
}

fn settings_page_name(p: &str) -> String {
    if ["editor", "browser", "packs", "settings"].contains(&p) { p.into() } else { "editor".into() }
}

fn main() -> glib::ExitCode {
    let mut flags = gio::ApplicationFlags::HANDLES_OPEN;
    if std::env::var("SKINVAS_SHOT").is_ok() {
        // Screenshot runs must not hand over to an instance the user already has open.
        flags |= gio::ApplicationFlags::NON_UNIQUE;
    }
    let app = adw::Application::builder().application_id("io.github.skinvas").flags(flags).build();
    let ctx: Rc<RefCell<Option<Rc<AppCtx>>>> = Rc::default();
    let c = ctx.clone();
    app.connect_activate(move |app| {
        if c.borrow().is_none() {
            *c.borrow_mut() = Some(build(app));
        }
    });
    let c = ctx.clone();
    app.connect_open(move |app, files, _| {
        if c.borrow().is_none() {
            *c.borrow_mut() = Some(build(app));
        }
        if let Some(p) = files.first().and_then(|f| f.path()) {
            c.borrow().as_ref().unwrap().load_path(&p);
        }
    });
    app.run()
}
