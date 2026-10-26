use crate::editor::{from_rgba, rgba};
use crate::i18n::{t, LANGS};
use crate::settings::{parse_hex, to_hex, Settings};
use crate::AppCtx;
use adw::prelude::*;
use std::rc::Rc;

pub const ACCENTS: [&str; 8] = ["#4caf50", "#3584e4", "#9141ac", "#e66100", "#e01b24", "#2ec27e", "#f5c211", "#e04f9f"];

/// Saves, re-applies the look and redraws. `rebuild` re-creates the whole UI (language change).
pub fn commit(ctx: &Rc<AppCtx>, f: impl FnOnce(&mut Settings), rebuild: bool) {
    {
        let mut s = ctx.st.borrow_mut();
        f(&mut s.settings);
        s.settings.save();
        s.refresh();
    }
    ctx.apply_look();
    if rebuild {
        let c = ctx.clone();
        gtk::glib::idle_add_local_once(move || c.rebuild());
    }
}

fn get<T>(ctx: &Rc<AppCtx>, f: impl Fn(&Settings) -> T) -> T {
    f(&ctx.st.borrow().settings)
}

fn switch(g: &adw::PreferencesGroup, ctx: &Rc<AppCtx>, key: &str, get_: fn(&Settings) -> bool, set: fn(&mut Settings, bool)) {
    let r = adw::SwitchRow::builder().title(t(key)).active(get(ctx, get_)).build();
    let c = ctx.clone();
    r.connect_active_notify(move |r| commit(&c, |s| set(s, r.is_active()), false));
    g.add(&r);
}

fn spin(g: &adw::PreferencesGroup, ctx: &Rc<AppCtx>, key: &str, range: (f64, f64, f64), digits: u32, get_: fn(&Settings) -> f64, set: fn(&mut Settings, f64)) -> adw::SpinRow {
    let r = adw::SpinRow::with_range(range.0, range.1, range.2);
    r.set_title(t(key));
    r.set_digits(digits);
    r.set_value(get(ctx, get_));
    let c = ctx.clone();
    r.connect_value_notify(move |r| commit(&c, |s| set(s, r.value()), false));
    g.add(&r);
    r
}

fn color(g: &adw::PreferencesGroup, ctx: &Rc<AppCtx>, key: &str, get_: fn(&Settings) -> String, set: fn(&mut Settings, String)) {
    let r = adw::ActionRow::builder().title(t(key)).build();
    let b = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::builder().with_alpha(true).build()));
    b.set_valign(gtk::Align::Center);
    b.set_rgba(&rgba(parse_hex(&get(ctx, get_))));
    let c = ctx.clone();
    b.connect_rgba_notify(move |b| commit(&c, |s| set(s, to_hex(from_rgba(&b.rgba()))), false));
    r.add_suffix(&b);
    g.add(&r);
}

fn combo(g: &adw::PreferencesGroup, ctx: &Rc<AppCtx>, key: &str, items: &[&str], selected: u32, on: impl Fn(&Rc<AppCtx>, u32) + 'static) {
    let r = adw::ComboRow::builder().title(t(key)).model(&gtk::StringList::new(items)).selected(selected).build();
    let c = ctx.clone();
    r.connect_selected_notify(move |r| on(&c, r.selected()));
    g.add(&r);
}

fn group(page: &adw::PreferencesPage, key: &str) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder().title(t(key)).build();
    page.add(&g);
    g
}

fn cache_label() -> String {
    format!("{} · {:.1} MB", t("cache_tip"), crate::net::cache_size() as f64 / 1e6)
}

fn account_group(page: &adw::PreferencesPage, ctx: &Rc<AppCtx>) {
    use crate::browser::fetch;
    use crate::library::Account;
    let g = group(page, "ely_account");
    match Account::load() {
        Some(acc) => {
            g.add(&adw::ActionRow::builder().title(format!("{} {}", t("logged_in_as"), acc.name)).subtitle("Ely.by").build());
            let row = gtk::Box::builder().spacing(8).halign(gtk::Align::Center).margin_top(8).build();
            let my = gtk::Button::with_label(t("my_skin"));
            let upload = gtk::Button::builder().label(t("upload_ely")).css_classes(["suggested-action"]).build();
            let out = gtk::Button::builder().label(t("logout")).css_classes(["destructive-action"]).build();
            let (c, name) = (ctx.clone(), acc.name.clone());
            my.connect_clicked(move |_| {
                let (c2, n) = (c.clone(), name.clone());
                fetch(
                    move || {
                        let f = crate::net::nick(crate::net::Source::ElyNick, &n)?.ok_or(t("nothing_found"))?;
                        let img = crate::skin::load(&crate::net::image(&f.url)?).map_err(|d| format!("{} {d}", t("bad_size")))?;
                        Ok::<_, String>((img, f.slim))
                    },
                    move |r| match r {
                        Ok((img, slim)) => {
                            let slim = slim.unwrap_or_else(|| crate::skin::detect_slim(&img));
                            c2.open_skin(img, slim, "Ely.by");
                        }
                        Err(e) => c2.toast(&format!("{}: {e}", t("error"))),
                    },
                );
            });
            // Ely.by has no upload API: save the PNG and open the site's upload form.
            let c = ctx.clone();
            upload.connect_clicked(move |_| {
                let dir = gtk::glib::user_special_dir(gtk::glib::UserDirectory::Pictures).unwrap_or_else(gtk::glib::home_dir).join("Skinvas");
                let path = dir.join("skinvas-ely.png");
                let r = std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|_| c.st.borrow().skin.save_with_format(&path, image::ImageFormat::Png).map_err(|e| e.to_string()));
                match r {
                    Ok(()) => {
                        gtk::UriLauncher::new("https://ely.by/skins/add").launch(Some(&c.win), None::<&gtk::gio::Cancellable>, |_| {});
                        c.toast(&format!("{} {}", t("upload_hint"), path.display()));
                    }
                    Err(e) => c.toast(&format!("{}: {e}", t("error"))),
                }
            });
            let c = ctx.clone();
            out.connect_clicked(move |_| {
                let a = acc.clone();
                fetch(move || crate::net::ely_logout(&a.token, &a.client_token), |_| {});
                Account::delete();
                let c2 = c.clone();
                gtk::glib::idle_add_local_once(move || c2.rebuild());
            });
            row.append(&my);
            row.append(&upload);
            row.append(&out);
            g.add(&row);
        }
        None => {
            g.set_description(Some(t("ely_tip")));
            let user = adw::EntryRow::builder().title(t("login")).build();
            let pass = adw::PasswordEntryRow::builder().title(t("password")).build();
            let totp = adw::EntryRow::builder().title(t("totp")).build();
            let btn = gtk::Button::builder().label(t("sign_in")).css_classes(["suggested-action"]).halign(gtk::Align::Center).margin_top(8).build();
            g.add(&user);
            g.add(&pass);
            g.add(&totp);
            g.add(&btn);
            let c = ctx.clone();
            btn.connect_clicked(move |b| {
                let (u, code) = (user.text().trim().to_string(), totp.text().trim().to_string());
                // Ely.by expects "password:code" for accounts with two-factor auth.
                let p = if code.is_empty() { pass.text().to_string() } else { format!("{}:{code}", pass.text()) };
                if u.is_empty() || p.is_empty() {
                    return;
                }
                b.set_sensitive(false);
                let client = format!("{:032x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
                let (c2, b2) = (c.clone(), b.clone());
                fetch(
                    move || crate::net::ely_login(&u, &p, &client).map(|(token, name, uuid)| Account { name, uuid, token, client_token: client }),
                    move |r| {
                        b2.set_sensitive(true);
                        match r.and_then(|a| a.save().map(|_| a)) {
                            Ok(a) => {
                                c2.toast(&format!("{} {}", t("logged_in_as"), a.name));
                                let c3 = c2.clone();
                                gtk::glib::idle_add_local_once(move || c3.rebuild());
                            }
                            Err(e) => c2.toast(&format!("{}: {e}", t("error"))),
                        }
                    },
                );
            });
        }
    }
}

pub fn page(ctx: &Rc<AppCtx>) -> gtk::Widget {
    let page = adw::PreferencesPage::new();

    let g = group(&page, "general");
    let lang = get(ctx, |s| s.lang.clone());
    let names: Vec<&str> = LANGS.iter().map(|l| l.1).collect();
    combo(&g, ctx, "language", &names, LANGS.iter().position(|l| l.0 == lang).unwrap_or(0) as u32, |c, i| {
        let code = LANGS[i as usize].0;
        crate::i18n::set_lang(code);
        commit(c, |s| s.lang = code.into(), true);
    });
    const THEMES: [&str; 4] = ["system", "light", "dark", "amoled"];
    let theme = get(ctx, |s| s.theme.clone());
    combo(&g, ctx, "theme", &[t("theme_system"), t("theme_light"), t("theme_dark"), t("theme_amoled")], THEMES.iter().position(|v| *v == theme).unwrap_or(2) as u32, |c, i| {
        commit(c, |s| s.theme = THEMES[i as usize].into(), false)
    });

    // Accent: presets + custom colour.
    let row = adw::ActionRow::builder().title(t("accent")).build();
    let custom = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
    custom.set_tooltip_text(Some(t("accent_custom")));
    custom.set_valign(gtk::Align::Center);
    custom.set_rgba(&rgba(parse_hex(&get(ctx, |s| s.accent.clone()))));
    for (i, hex) in ACCENTS.iter().enumerate() {
        let b = gtk::Button::new();
        b.add_css_class("sv-sw");
        b.add_css_class(&format!("sv-sw-{i}"));
        b.set_valign(gtk::Align::Center);
        let cb = custom.clone();
        b.connect_clicked(move |_| cb.set_rgba(&rgba(parse_hex(hex))));
        row.add_suffix(&b);
    }
    let c = ctx.clone();
    custom.connect_rgba_notify(move |b| {
        let mut col = from_rgba(&b.rgba());
        col[3] = 255;
        commit(&c, |s| s.accent = to_hex(col)[..7].into(), false)
    });
    row.add_suffix(&custom);
    g.add(&row);
    spin(&g, ctx, "radius", (0., 24., 1.), 0, |s| s.radius, |s, v| s.radius = v);
    spin(&g, ctx, "font_scale", (0.75, 1.5, 0.05), 2, |s| s.font_scale, |s, v| s.font_scale = v);
    switch(&g, ctx, "animations", |s| s.animations, |s, v| s.animations = v);
    switch(&g, ctx, "tablet", |s| s.tablet, |s, v| s.tablet = v);
    let tour = adw::ButtonRow::builder().title(t("show_tour")).build();
    let c = ctx.clone();
    tour.connect_activated(move |_| crate::tour::show(&c));
    g.add(&tour);

    let g = group(&page, "editor_group");
    switch(&g, ctx, "show_grid", |s| s.show_grid, |s, v| s.show_grid = v);
    color(&g, ctx, "grid_color", |s| s.grid_color.clone(), |s, v| s.grid_color = v);
    switch(&g, ctx, "checker", |s| s.checker, |s, v| s.checker = v);
    switch(&g, ctx, "right_click_picks", |s| s.right_click_picks, |s, v| s.right_click_picks = v);
    spin(&g, ctx, "undo_limit", (10., 2000., 10.), 0, |s| s.undo_limit as f64, |s, v| s.undo_limit = v as u32);
    switch(&g, ctx, "confirm_unsaved", |s| s.confirm_unsaved, |s, v| s.confirm_unsaved = v);
    spin(&g, ctx, "autosave", (0., 600., 15.), 0, |s| s.autosave_secs as f64, |s, v| s.autosave_secs = v as u32).set_subtitle(t("autosave_tip"));
    spin(&g, ctx, "palette_size", (12., 48., 1.), 0, |s| s.palette_size, |s, v| s.palette_size = v);
    spin(&g, ctx, "picker_size", (160., 520., 10.), 0, |s| s.picker_size, |s, v| s.picker_size = v);

    let g = group(&page, "browser_group");
    let cache = adw::SwitchRow::builder().title(t("cache")).subtitle(cache_label()).active(get(ctx, |s| s.cache)).build();
    let c = ctx.clone();
    cache.connect_active_notify(move |r| commit(&c, |s| s.cache = r.is_active(), false));
    let clear = gtk::Button::builder().label(t("cache_clear")).valign(gtk::Align::Center).build();
    let cr = cache.clone();
    clear.connect_clicked(move |_| {
        crate::net::clear_cache();
        cr.set_subtitle(&cache_label());
    });
    cache.add_suffix(&clear);
    g.add(&cache);
    switch(&g, ctx, "safe", |s| s.safe, |s, v| s.safe = v);

    account_group(&page, ctx);

    let g = group(&page, "integrations");
    let r = adw::SwitchRow::builder().title(t("discord")).subtitle(t("discord_tip")).active(get(ctx, |s| s.discord)).build();
    let c = ctx.clone();
    r.connect_active_notify(move |r| {
        commit(&c, |s| s.discord = r.is_active(), false);
        c.update_presence();
    });
    g.add(&r);
    let id = adw::EntryRow::builder().title(t("discord_id")).text(get(ctx, |s| s.discord_id.clone())).show_apply_button(true).build();
    let c = ctx.clone();
    id.connect_apply(move |e| {
        let v: String = e.text().chars().filter(|c| c.is_ascii_digit()).collect();
        commit(&c, |s| s.discord_id = v, false);
        c.update_presence();
    });
    g.add(&id);

    let g = group(&page, "viewer_group");
    switch(&g, ctx, "slim_default", |s| s.slim_default, |s, v| s.slim_default = v);
    switch(&g, ctx, "layers3d_default", |s| s.layers3d, |s, v| s.layers3d = v);
    switch(&g, ctx, "auto_rotate", |s| s.auto_rotate, |s, v| s.auto_rotate = v);
    switch(&g, ctx, "walk_default", |s| s.walk, |s, v| s.walk = v);
    spin(&g, ctx, "rotate_speed", (0.1, 3., 0.1), 1, |s| s.rotate_speed, |s, v| s.rotate_speed = v);
    spin(&g, ctx, "walk_speed", (0.2, 3., 0.1), 1, |s| s.walk_speed, |s, v| s.walk_speed = v);
    spin(&g, ctx, "thickness", (0.25, 2., 0.25), 2, |s| s.thickness, |s, v| s.thickness = v);
    spin(&g, ctx, "fov", (20., 90., 1.), 0, |s| s.fov, |s, v| s.fov = v);
    switch(&g, ctx, "shading", |s| s.shading, |s, v| s.shading = v);
    color(&g, ctx, "bg", |s| s.bg.clone(), |s, v| s.bg = v);
    spin(&g, ctx, "quality", (0.25, 2., 0.25), 2, |s| s.quality, |s, v| s.quality = v).set_subtitle(t("quality_tip"));

    let g = adw::PreferencesGroup::new();
    let reset = gtk::Button::with_label(t("reset_settings"));
    reset.add_css_class("destructive-action");
    reset.set_halign(gtk::Align::Center);
    let c = ctx.clone();
    reset.connect_clicked(move |_| {
        let lang = get(&c, |s| s.lang.clone());
        commit(&c, |s| *s = Settings { lang, ..Settings::default() }, true);
    });
    g.add(&reset);
    page.add(&g);
    page.upcast()
}
