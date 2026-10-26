// Big inline colour picker in a popover: SV square, hue bar, alpha bar, hex entry.
use crate::editor::{from_rgba, rgba};
use crate::settings::{parse_hex, to_hex};
use gtk::{cairo, gdk, prelude::*};
use std::{cell::Cell, rc::Rc};

pub fn hsv_to_rgb(h: f64, s: f64, v: f64) -> [f64; 3] {
    let c = v * s;
    let hp = (h.rem_euclid(360.)) / 60.;
    let x = c * (1. - (hp % 2. - 1.).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

pub fn rgb_to_hsv(r: f64, g: f64, b: f64) -> [f64; 3] {
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let h = if d == 0. {
        0.
    } else if max == r {
        60. * ((g - b) / d).rem_euclid(6.)
    } else if max == g {
        60. * ((b - r) / d + 2.)
    } else {
        60. * ((r - g) / d + 4.)
    };
    [h, if max == 0. { 0. } else { d / max }, max]
}

#[derive(Clone)]
pub struct Picker {
    pub button: gtk::MenuButton,
    hsva: Rc<Cell<[f64; 4]>>,
    redraw: Rc<dyn Fn()>,
}

impl Picker {
    /// `size` is read every time the popover opens, so the setting applies live.
    pub fn new(initial: [u8; 4], size: impl Fn() -> i32 + 'static, on_change: impl Fn([u8; 4]) + 'static) -> Self {
        let c = rgba(initial);
        let [h, s, v] = rgb_to_hsv(c.red() as f64, c.green() as f64, c.blue() as f64);
        let hsva = Rc::new(Cell::new([h, s, v, c.alpha() as f64]));
        let current = {
            let hsva = hsva.clone();
            move || {
                let [h, s, v, a] = hsva.get();
                let [r, g, b] = hsv_to_rgb(h, s, v);
                [r, g, b, a].map(|x| (x * 255.).round() as u8)
            }
        };

        let swatch = gtk::DrawingArea::builder().content_width(40).content_height(24).build();
        let cur = current.clone();
        swatch.set_draw_func(move |_, cr, w, h| {
            checker(cr, w as f64, h as f64, 6.);
            let c = cur();
            cr.set_source_rgba(c[0] as f64 / 255., c[1] as f64 / 255., c[2] as f64 / 255., c[3] as f64 / 255.);
            cr.paint().ok();
        });

        let sv = gtk::DrawingArea::new();
        let hue = gtk::DrawingArea::builder().content_height(22).build();
        let alpha = gtk::DrawingArea::builder().content_height(22).build();
        let hex = gtk::Entry::builder().max_length(9).width_chars(10).build();
        let big = gtk::DrawingArea::builder().content_height(40).hexpand(true).build();

        let hs = hsva.clone();
        sv.set_draw_func(move |_, cr, w, h| {
            let [hh, s, v, _] = hs.get();
            let [r, g, b] = hsv_to_rgb(hh, 1., 1.);
            let (w, h) = (w as f64, h as f64);
            let lg = cairo::LinearGradient::new(0., 0., w, 0.);
            lg.add_color_stop_rgb(0., 1., 1., 1.);
            lg.add_color_stop_rgb(1., r, g, b);
            cr.set_source(&lg).ok();
            cr.paint().ok();
            let dg = cairo::LinearGradient::new(0., 0., 0., h);
            dg.add_color_stop_rgba(0., 0., 0., 0., 0.);
            dg.add_color_stop_rgba(1., 0., 0., 0., 1.);
            cr.set_source(&dg).ok();
            cr.paint().ok();
            marker(cr, s * w, (1. - v) * h);
        });
        let hs = hsva.clone();
        hue.set_draw_func(move |_, cr, w, h| {
            let lg = cairo::LinearGradient::new(0., 0., w as f64, 0.);
            for i in 0..=6 {
                let [r, g, b] = hsv_to_rgb(i as f64 * 60., 1., 1.);
                lg.add_color_stop_rgb(i as f64 / 6., r, g, b);
            }
            cr.set_source(&lg).ok();
            cr.paint().ok();
            marker(cr, hs.get()[0] / 360. * w as f64, h as f64 / 2.);
        });
        let (hs, cur) = (hsva.clone(), current.clone());
        alpha.set_draw_func(move |_, cr, w, h| {
            checker(cr, w as f64, h as f64, 6.);
            let c = cur();
            let lg = cairo::LinearGradient::new(0., 0., w as f64, 0.);
            lg.add_color_stop_rgba(0., c[0] as f64 / 255., c[1] as f64 / 255., c[2] as f64 / 255., 0.);
            lg.add_color_stop_rgba(1., c[0] as f64 / 255., c[1] as f64 / 255., c[2] as f64 / 255., 1.);
            cr.set_source(&lg).ok();
            cr.paint().ok();
            marker(cr, hs.get()[3] * w as f64, h as f64 / 2.);
        });
        let cur = current.clone();
        big.set_draw_func(move |_, cr, w, h| {
            checker(cr, w as f64, h as f64, 8.);
            let c = cur();
            cr.set_source_rgba(c[0] as f64 / 255., c[1] as f64 / 255., c[2] as f64 / 255., c[3] as f64 / 255.);
            cr.paint().ok();
        });

        let syncing = Rc::new(Cell::new(false));
        let redraw: Rc<dyn Fn()> = {
            let (sw, sv, hue, alpha, big, hex, cur, syncing) = (swatch.clone(), sv.clone(), hue.clone(), alpha.clone(), big.clone(), hex.clone(), current.clone(), syncing.clone());
            Rc::new(move || {
                for w in [&sw, &sv, &hue, &alpha, &big] {
                    w.queue_draw();
                }
                syncing.set(true);
                let c = cur();
                hex.set_text(&if c[3] == 255 { to_hex(c)[..7].to_string() } else { to_hex(c) });
                syncing.set(false);
            })
        };
        let on_change: Rc<dyn Fn([u8; 4])> = Rc::new(on_change);
        let changed = {
            let (redraw, on_change, cur) = (redraw.clone(), on_change.clone(), current.clone());
            Rc::new(move || {
                redraw();
                on_change(cur());
            })
        };

        // Drag handlers: each maps pointer position to the hsva channels it controls.
        let drag = |area: &gtk::DrawingArea, f: fn(&mut [f64; 4], f64, f64)| {
            let g = gtk::GestureDrag::new();
            let (a, hs, ch) = (area.clone(), hsva.clone(), changed.clone());
            let apply = move |x: f64, y: f64| {
                let mut v = hs.get();
                f(&mut v, (x / a.width() as f64).clamp(0., 1.), (y / a.height() as f64).clamp(0., 1.));
                hs.set(v);
                ch();
            };
            let ap = apply.clone();
            g.connect_drag_begin(move |_, x, y| ap(x, y));
            g.connect_drag_update(move |g, dx, dy| {
                if let Some((x, y)) = g.start_point() {
                    apply(x + dx, y + dy);
                }
            });
            area.add_controller(g);
        };
        drag(&sv, |v, x, y| (v[1], v[2]) = (x, 1. - y));
        drag(&hue, |v, x, _| v[0] = x * 360.);
        drag(&alpha, |v, x, _| v[3] = x);

        let (hs, ch, sy) = (hsva.clone(), changed.clone(), syncing.clone());
        hex.connect_changed(move |e| {
            let t = e.text();
            let t = t.trim_start_matches('#');
            if sy.get() || !(t.len() == 6 || t.len() == 8) || !t.chars().all(|c| c.is_ascii_hexdigit()) {
                return;
            }
            let c = parse_hex(t);
            let [h, s, v] = rgb_to_hsv(c[0] as f64 / 255., c[1] as f64 / 255., c[2] as f64 / 255.);
            hs.set([h, s, v, c[3] as f64 / 255.]);
            ch();
        });

        let col = gtk::Box::new(gtk::Orientation::Vertical, 10);
        col.set_margin_start(6);
        col.set_margin_end(6);
        col.set_margin_top(6);
        col.set_margin_bottom(6);
        col.append(&sv);
        col.append(&hue);
        col.append(&alpha);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&big);
        row.append(&hex);
        col.append(&row);
        let pop = gtk::Popover::builder().child(&col).build();
        let (sv2, hue2, alpha2) = (sv.clone(), hue.clone(), alpha.clone());
        pop.connect_show(move |_| {
            let s = size().clamp(160, 520);
            sv2.set_content_width(s);
            sv2.set_content_height(s);
            hue2.set_content_width(s);
            alpha2.set_content_width(s);
        });
        let button = gtk::MenuButton::builder().child(&swatch).popover(&pop).valign(gtk::Align::Center).build();
        redraw();
        Picker { button, hsva, redraw }
    }

    /// Same name as `ColorDialogButton::set_rgba`, so callers did not have to change.
    pub fn set_rgba(&self, c: &gdk::RGBA) {
        let [h, s, v] = rgb_to_hsv(c.red() as f64, c.green() as f64, c.blue() as f64);
        // Keep the hue when the new colour is grey, so the SV square doesn't jump to red.
        let h = if s == 0. { self.hsva.get()[0] } else { h };
        self.hsva.set([h, s, v, c.alpha() as f64]);
        (self.redraw)();
    }

    #[allow(dead_code)]
    pub fn rgba(&self) -> [u8; 4] {
        let [h, s, v, a] = self.hsva.get();
        let [r, g, b] = hsv_to_rgb(h, s, v);
        from_rgba(&gdk::RGBA::new(r as f32, g as f32, b as f32, a as f32))
    }
}

fn marker(cr: &cairo::Context, x: f64, y: f64) {
    cr.new_sub_path();
    cr.arc(x, y, 6., 0., std::f64::consts::TAU);
    cr.set_source_rgb(0., 0., 0.);
    cr.set_line_width(3.);
    cr.stroke_preserve().ok();
    cr.set_source_rgb(1., 1., 1.);
    cr.set_line_width(1.5);
    cr.stroke().ok();
}

fn checker(cr: &cairo::Context, w: f64, h: f64, s: f64) {
    for y in 0..(h / s).ceil() as i32 {
        for x in 0..(w / s).ceil() as i32 {
            let v = if (x + y) % 2 == 0 { 0.8 } else { 0.6 };
            cr.set_source_rgb(v, v, v);
            cr.rectangle(x as f64 * s, y as f64 * s, s, s);
            cr.fill().ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_roundtrip() {
        for c in [[1., 0., 0.], [0.2, 0.6, 0.4], [0.5, 0.5, 0.5], [0., 0., 0.], [0.9, 0.1, 0.8]] {
            let [h, s, v] = rgb_to_hsv(c[0], c[1], c[2]);
            let r = hsv_to_rgb(h, s, v);
            for i in 0..3 {
                assert!((r[i] - c[i]).abs() < 1e-9, "{c:?} -> {r:?}");
            }
        }
    }
}
