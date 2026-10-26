// HTTP + optional disk cache for skin images. Runs on worker threads (gio::spawn_blocking).
use base64::Engine;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

const UA: &str = concat!("Skinvas/", env!("CARGO_PKG_VERSION"), " (+https://github.com/skinvas)");
pub static CACHE_ON: AtomicBool = AtomicBool::new(false);
pub static SAFE: AtomicBool = AtomicBool::new(true);

// shortcut: tag/name word list only, upgrade if a source ever exposes a real NSFW flag.
const BLOCK: [&str; 16] = ["hitler", "adolf", "nazi", "swastika", "porn", "sex", "nsfw", "nude", "naked", "hentai", "xxx", "panties", "boobs", "18+", "ahegao", "dick"];

pub fn blocked(text: &str) -> bool {
    let t = text.to_lowercase();
    SAFE.load(Ordering::Relaxed) && BLOCK.iter().any(|w| t.contains(w))
}

pub fn cache_dir() -> PathBuf {
    gtk::glib::user_cache_dir().join("skinvas")
}

fn cache_path(url: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    cache_dir().join(format!("{:016x}.png", h.finish()))
}

pub fn cache_size() -> u64 {
    std::fs::read_dir(cache_dir()).map(|d| d.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()).unwrap_or(0)
}

pub fn clear_cache() {
    let _ = std::fs::remove_dir_all(cache_dir());
}

pub fn get(url: &str) -> Result<Vec<u8>, String> {
    let mut r = ureq::get(url).header("User-Agent", UA).call().map_err(|e| e.to_string())?;
    r.body_mut().with_config().limit(8 << 20).read_to_vec().map_err(|e| e.to_string())
}

pub fn json(url: &str) -> Result<Value, String> {
    serde_json::from_slice(&get(url)?).map_err(|e| e.to_string())
}

pub fn post_json(url: &str, body: &Value) -> Result<Value, String> {
    let r = ureq::post(url).header("User-Agent", UA).config().http_status_as_error(false).build().header("Content-Type", "application/json").send(&serde_json::to_vec(body).unwrap());
    let mut r = r.map_err(|e| e.to_string())?;
    let ok = r.status().is_success();
    let text = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let v: Value = if text.trim().is_empty() { Value::Null } else { serde_json::from_str(&text).map_err(|e| e.to_string())? };
    if ok { Ok(v) } else { Err(v["errorMessage"].as_str().unwrap_or("HTTP error").to_string()) }
}

/// Skin PNGs are content-addressed on every source we use, so caching by URL is safe.
pub fn image(url: &str) -> Result<Vec<u8>, String> {
    if let Some(path) = url.strip_prefix("file://") {
        return std::fs::read(path).map_err(|e| e.to_string());
    }
    let cache = CACHE_ON.load(Ordering::Relaxed);
    let p = cache_path(url);
    if cache {
        if let Ok(b) = std::fs::read(&p) {
            return Ok(b);
        }
    }
    let b = get(url)?;
    if cache {
        let _ = std::fs::create_dir_all(cache_dir());
        let _ = std::fs::write(&p, &b);
    }
    Ok(b)
}

/// A skin found online.
#[derive(Clone, Debug)]
pub struct Found {
    pub name: String,
    pub url: String,
    pub slim: Option<bool>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Source {
    Ely,
    MineSkin,
    Mojang,
    ElyNick,
    /// Skins Minecraft and launchers cached on this computer.
    Local,
}

/// One page of results. `cursor` is the page number (Ely) or the "after" id (MineSkin).
pub fn search(src: Source, query: &str, cursor: &str) -> Result<(Vec<Found>, Option<String>), String> {
    let q = urlencode(query.trim());
    match src {
        Source::Ely => {
            let page: u32 = cursor.parse().unwrap_or(1);
            let tag = if q.is_empty() { String::new() } else { format!("&tags[]={q}") };
            let v = json(&format!("https://ely.by/api/legacy/skins?page={page}{tag}"))?;
            let items = v["items"].as_array().cloned().unwrap_or_default();
            let found = items
                .iter()
                .filter_map(|i| {
                    let tags: Vec<&str> = i["tags"].as_array().map(|t| t.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
                    Some(Found { name: if tags.is_empty() { format!("#{}", i["id"]) } else { tags.join(", ") }, url: i["skin_url"].as_str()?.to_string(), slim: i["is_slim"].as_bool() })
                })
                .filter(|f: &Found| !blocked(&f.name))
                .collect();
            let next = (v["next"].as_u64().is_some_and(|n| n as u32 > page)).then(|| (page + 1).to_string());
            Ok((found, next))
        }
        Source::MineSkin => {
            let mut url = "https://api.mineskin.org/v2/skins?size=36".to_string();
            if !q.is_empty() {
                url += &format!("&filter={q}");
            }
            if !cursor.is_empty() {
                url += &format!("&after={}", urlencode(cursor));
            }
            let v = json(&url)?;
            let found = v["skins"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| Some(Found { name: s["name"].as_str().unwrap_or("MineSkin").to_string(), url: format!("https://textures.minecraft.net/texture/{}", s["texture"].as_str()?), slim: None }))
                        .filter(|f| !blocked(&f.name))
                        .collect()
                })
                .unwrap_or_default();
            Ok((found, v["pagination"]["next"]["after"].as_str().map(String::from)))
        }
        Source::Mojang | Source::ElyNick => Ok((nick(src, query.trim())?.into_iter().collect(), None)),
        Source::Local => {
            let all = crate::tools::cached_skins();
            let from: usize = cursor.parse().unwrap_or(0);
            let page: Vec<Found> = all.iter().skip(from).take(40).map(|p| Found { name: p.file_name().unwrap_or_default().to_string_lossy().chars().take(12).collect(), url: format!("file://{}", p.display()), slim: None }).collect();
            let next = (from + 40 < all.len()).then(|| (from + 40).to_string());
            Ok((page, next))
        }
    }
}

/// Current skin of a player by nickname.
pub fn nick(src: Source, name: &str) -> Result<Option<Found>, String> {
    if name.is_empty() {
        return Ok(None);
    }
    let v = if src == Source::ElyNick {
        json(&format!("https://skinsystem.ely.by/textures/{}", urlencode(name)))?
    } else {
        let Ok(p) = json(&format!("https://api.mojang.com/users/profiles/minecraft/{}", urlencode(name))) else { return Ok(None) };
        let Some(id) = p["id"].as_str() else { return Ok(None) };
        let prof = json(&format!("https://sessionserver.mojang.com/session/minecraft/profile/{id}"))?;
        let b64 = prof["properties"].as_array().and_then(|a| a.iter().find(|p| p["name"] == "textures")).and_then(|p| p["value"].as_str()).unwrap_or("");
        let raw = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| e.to_string())?;
        serde_json::from_slice::<Value>(&raw).map_err(|e| e.to_string())?["textures"].clone()
    };
    let skin = &v["SKIN"];
    Ok(skin["url"].as_str().map(|u| Found { name: name.to_string(), url: u.replace("http://", "https://"), slim: Some(skin["metadata"]["model"] == "slim") }))
}

pub fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

// --- Ely.by account (Yggdrasil-compatible authserver) ---

pub fn ely_login(user: &str, pass: &str, client_token: &str) -> Result<(String, String, String), String> {
    let v = post_json("https://authserver.ely.by/auth/authenticate", &serde_json::json!({"username": user, "password": pass, "clientToken": client_token, "requestUser": true}))?;
    let tok = v["accessToken"].as_str().ok_or("no token")?.to_string();
    let p = &v["selectedProfile"];
    Ok((tok, p["name"].as_str().unwrap_or(user).to_string(), p["id"].as_str().unwrap_or("").to_string()))
}

pub fn ely_logout(token: &str, client_token: &str) {
    let _ = post_json("https://authserver.ely.by/auth/invalidate", &serde_json::json!({"accessToken": token, "clientToken": client_token}));
}

#[cfg(test)]
mod tests {
    #[test]
    fn blocklist() {
        assert!(super::blocked("Adolf Hitler, WW2"));
        assert!(!super::blocked("Cat, Cute, Girl"));
    }

    #[test]
    fn urlencode_escapes() {
        assert_eq!(super::urlencode("a b/ы"), "a%20b%2F%D1%8B");
    }
}

/// Uploads a skin to MineSkin (no key needed; ~10 per minute). Returns (share link, texture URL).
pub fn mineskin_upload(png: &[u8], name: &str, slim: bool) -> Result<(String, String), String> {
    let boundary = "----skinvas7d1f9a";
    let mut body = Vec::new();
    for (k, v) in [("name", name), ("variant", if slim { "slim" } else { "classic" }), ("visibility", "public")] {
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
    }
    body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"skin.png\"\r\nContent-Type: image/png\r\n\r\n").as_bytes());
    body.extend(png);
    body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
    let mut r = ureq::post("https://api.mineskin.org/v2/generate")
        .header("User-Agent", UA)
        .header("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
        .config()
        .http_status_as_error(false)
        .build()
        .send(&body[..])
        .map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&r.body_mut().read_to_vec().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if v["success"] != true {
        return Err(v["errors"][0]["message"].as_str().unwrap_or("MineSkin error").to_string());
    }
    let s = &v["skin"];
    let link = s["shortId"].as_str().map(|id| format!("https://minesk.in/{id}")).unwrap_or_default();
    let tex = s["texture"]["url"]["skin"].as_str().unwrap_or("").replace("http://", "https://");
    Ok((link, tex))
}
