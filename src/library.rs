// Local skin library (collections + tags), friends list with local skin history, Ely.by account.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub collection: String,
    pub tags: Vec<String>,
    pub slim: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Friend {
    pub nick: String,
    pub ely: bool,
    /// Skin URLs seen for this nick, newest first (Mojang has no public history API).
    pub history: Vec<String>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Library {
    pub entries: Vec<Entry>,
    pub collections: Vec<String>,
    pub friends: Vec<Friend>,
}

pub fn dir() -> PathBuf {
    gtk::glib::user_data_dir().join("skinvas")
}

impl Library {
    pub fn load(default_collections: &[&str]) -> Self {
        let mut l: Library = std::fs::read(dir().join("library.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        if l.collections.is_empty() {
            l.collections = default_collections.iter().map(|s| s.to_string()).collect();
        }
        l
    }

    pub fn save(&self) -> Result<(), String> {
        std::fs::create_dir_all(dir().join("skins")).map_err(|e| e.to_string())?;
        // Write-then-rename so a crash never leaves a half-written library.
        let tmp = dir().join("library.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap()).map_err(|e| e.to_string())?;
        std::fs::rename(tmp, dir().join("library.json")).map_err(|e| e.to_string())
    }

    pub fn png_path(id: &str) -> PathBuf {
        dir().join("skins").join(format!("{id}.png"))
    }

    pub fn add(&mut self, png: &[u8], name: &str, slim: bool, collection: &str) -> Result<(), String> {
        let id = format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        std::fs::create_dir_all(dir().join("skins")).map_err(|e| e.to_string())?;
        std::fs::write(Self::png_path(&id), png).map_err(|e| e.to_string())?;
        self.entries.insert(0, Entry { id, name: name.into(), collection: collection.into(), tags: vec![], slim });
        self.save()
    }

    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        self.entries.retain(|e| e.id != id);
        let _ = std::fs::remove_file(Self::png_path(id));
        self.save()
    }

    /// Records a newly seen skin URL; returns true if it is new.
    pub fn saw(&mut self, nick: &str, url: &str) -> bool {
        let Some(f) = self.friends.iter_mut().find(|f| f.nick.eq_ignore_ascii_case(nick)) else { return false };
        if f.history.first().map(|u| u == url).unwrap_or(false) {
            return false;
        }
        f.history.retain(|u| u != url);
        f.history.insert(0, url.into());
        true
    }
}

pub fn parse_tags(s: &str) -> Vec<String> {
    let mut v: Vec<String> = s.split(',').map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()).collect();
    v.dedup();
    v
}

pub fn matches(e: &Entry, collection: Option<&str>, q: &str) -> bool {
    let q = q.trim().to_lowercase();
    collection.is_none_or(|c| e.collection == c) && (q.is_empty() || e.name.to_lowercase().contains(&q) || e.tags.iter().any(|t| t.contains(&q)))
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Account {
    pub name: String,
    pub uuid: String,
    pub token: String,
    pub client_token: String,
}

fn account_path() -> PathBuf {
    gtk::glib::user_config_dir().join("skinvas").join("account.json")
}

impl Account {
    pub fn load() -> Option<Self> {
        std::fs::read(account_path()).ok().and_then(|b| serde_json::from_slice(&b).ok())
    }

    pub fn save(&self) -> Result<(), String> {
        use std::io::Write;
        let p = account_path();
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        // The token is a credential: owner-only permissions (Windows profiles are per-user already).
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let mut f = o.open(&p).map_err(|e| e.to_string())?;
        f.write_all(&serde_json::to_vec(self).unwrap()).map_err(|e| e.to_string())
    }

    pub fn delete() {
        let _ = std::fs::remove_file(account_path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_filter_and_history() {
        assert_eq!(parse_tags(" Knight, , RED ,red"), vec!["knight", "red"]);
        let e = Entry { id: "1".into(), name: "Cool Knight".into(), collection: "liked".into(), tags: vec!["armor".into()], slim: false };
        assert!(matches(&e, Some("liked"), "knight"));
        assert!(matches(&e, None, "arm"));
        assert!(!matches(&e, Some("mine"), ""));
        let mut l = Library { friends: vec![Friend { nick: "Bob".into(), ely: false, history: vec![] }], ..Default::default() };
        assert!(l.saw("bob", "u1"));
        assert!(!l.saw("bob", "u1"));
        assert!(l.saw("bob", "u2"));
        assert!(l.saw("bob", "u1"));
        assert_eq!(l.friends[0].history, vec!["u1", "u2"]);
    }
}
