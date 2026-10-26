// Lua plugins: ~/.local/share/skinvas/plugins/*.lua, each defines `run(skin)`.
// Sandboxed: only table/string/math/utf8 libs (no io, os, require) and an instruction budget.
use image::{Rgba, RgbaImage};
use mlua::{HookTriggers, Lua, LuaOptions, StdLib, UserData, UserDataFields, UserDataMethods, VmState};
use std::path::PathBuf;

pub fn dir() -> PathBuf {
    crate::library::dir().join("plugins")
}

const EXAMPLES: [(&str, &str); 3] = [
    ("grayscale.lua", r#"name = "Grayscale / Ч/Б"
function run(skin)
  for y = 0, skin.height - 1 do
    for x = 0, skin.width - 1 do
      local r, g, b, a = skin:get(x, y)
      local v = math.floor(0.299 * r + 0.587 * g + 0.114 * b)
      skin:set(x, y, v, v, v, a)
    end
  end
end
"#),
    ("invert.lua", r#"name = "Invert / Инверсия"
function run(skin)
  for y = 0, 63 do
    for x = 0, 63 do
      local r, g, b, a = skin:get(x, y)
      if a > 0 then skin:set(x, y, 255 - r, 255 - g, 255 - b, a) end
    end
  end
end
"#),
    ("noise.lua", r#"name = "Fabric noise / Шум ткани"
-- `slim` is true for slim (3px arm) skins.
function run(skin)
  for y = 0, 63 do
    for x = 0, 63 do
      local r, g, b, a = skin:get(x, y)
      if a > 0 then
        local n = math.random(-10, 10)
        local c = function(v) return math.max(0, math.min(255, v + n)) end
        skin:set(x, y, c(r), c(g), c(b), a)
      end
    end
  end
end
"#),
];

/// Lists (file, display name); writes the examples the first time.
pub fn list() -> Vec<(PathBuf, String)> {
    let d = dir();
    if !d.exists() && std::fs::create_dir_all(&d).is_ok() {
        for (f, src) in EXAMPLES {
            let _ = std::fs::write(d.join(f), src);
        }
    }
    let mut v: Vec<(PathBuf, String)> = std::fs::read_dir(&d)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "lua")).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            // Display name: first line `name = "..."`, else the file name.
            let src = std::fs::read_to_string(&p).unwrap_or_default();
            let name = src.lines().find_map(|l| l.trim().strip_prefix("name").map(|r| r.trim_start_matches([' ', '=']).trim().trim_matches('"').to_string()));
            let stem = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            (p, name.filter(|n| !n.is_empty()).unwrap_or(stem))
        })
        .collect();
    v.sort_by(|a, b| a.1.cmp(&b.1));
    v
}

struct Skin(RgbaImage);

impl UserData for Skin {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("width", |_, s| Ok(s.0.width()));
        f.add_field_method_get("height", |_, s| Ok(s.0.height()));
    }
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("get", |_, s, (x, y): (u32, u32)| {
            let p = s.0.get_pixel_checked(x, y).map(|p| p.0).unwrap_or([0; 4]);
            Ok((p[0], p[1], p[2], p[3]))
        });
        m.add_method_mut("set", |_, s, (x, y, r, g, b, a): (u32, u32, f64, f64, f64, Option<f64>)| {
            let c = |v: f64| v.round().clamp(0., 255.) as u8;
            if x < s.0.width() && y < s.0.height() {
                s.0.put_pixel(x, y, Rgba([c(r), c(g), c(b), c(a.unwrap_or(255.))]));
            }
            Ok(())
        });
    }
}

/// Runs `run(skin)` from the plugin source and returns the changed skin.
pub fn run(src: &str, img: &RgbaImage, slim: bool) -> Result<RgbaImage, String> {
    let lua = Lua::new_with(StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8, LuaOptions::default()).map_err(|e| e.to_string())?;
    // ~200M instructions is far more than any per-pixel script needs; stops endless loops.
    let budget = std::cell::Cell::new(0u32);
    lua.set_hook(HookTriggers::new().every_nth_instruction(100_000), move |_, _| {
        budget.set(budget.get() + 1);
        if budget.get() > 2000 { Err(mlua::Error::runtime("plugin took too long")) } else { Ok(VmState::Continue) }
    })
    .map_err(|e| e.to_string())?;
    lua.globals().set("slim", slim).map_err(|e| e.to_string())?;
    lua.load(src).exec().map_err(|e| e.to_string())?;
    let f: mlua::Function = lua.globals().get("run").map_err(|_| "no run(skin) function".to_string())?;
    let ud = lua.create_userdata(Skin(img.clone())).map_err(|e| e.to_string())?;
    f.call::<()>(ud.clone()).map_err(|e| e.to_string())?;
    let out = ud.take::<Skin>().map_err(|e| e.to_string())?.0;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples_run_and_sandbox_holds() {
        let img = crate::skin::default_skin();
        let g = run(EXAMPLES[0].1, &img, false).unwrap();
        let p = g.get_pixel(20, 20);
        assert!(p[0] == p[1] && p[1] == p[2]);
        assert!(run("function run(s) os.execute('true') end", &img, false).is_err());
        assert!(run("function run(s) io.open('/etc/passwd') end", &img, false).is_err());
        assert!(run("function run(s) while true do end end", &img, false).unwrap_err().contains("too long"));
    }
}
