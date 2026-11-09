//! Translation memory.
//!
//! Two stores share one type:
//! * the automatic cache, scoped by profile, title and source language, then
//!   keyed by (model, kind, source); old unscoped entries are not reused;
//! * user corrections, keyed by source only — always win over the model.
//!
//! Persistence is append-only TSV (`key fields… <TAB> translation`): crash-safe,
//! human-readable, and the last line for a key wins on reload.

use crate::tsv;
use crate::lang::Lang;
use std::collections::HashMap;

/// Identity of the reading whose automatic translations may be reused.
/// User corrections remain separate: their keys are intentionally source-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub profile: String,
    pub title: String,
    pub lang: Lang,
}

impl Scope {
    pub fn new(profile: &str, title: &str, lang: Lang) -> Self {
        Self { profile: profile.to_string(), title: title.trim().to_string(), lang }
    }

    pub fn key<'a>(&'a self, model: &'a str, kind: &'a str, source: &'a str) -> [&'a str; 7] {
        ["v2", &self.profile, &self.title, self.lang.code(), model, kind, source]
    }
}

#[derive(Debug, Default)]
pub struct Store {
    map: HashMap<String, String>,
}

fn key(parts: &[&str]) -> String {
    parts.join("\u{1f}")
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop the entries of the old cache format (without a reading scope):
    /// they are never looked up any more. Returns how many went.
    pub fn drop_unscoped(&mut self) -> usize {
        let before = self.map.len();
        let mark = format!("v2{}", '\u{1f}');
        self.map.retain(|k, _| k.starts_with(&mark));
        before - self.map.len()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Load TSV where the last column is the value and all previous columns form the key.
    pub fn load_tsv(&mut self, content: &str) -> usize {
        let mut n = 0;
        for row in tsv::parse(content) {
            if row.len() >= 2 {
                let (val, k) = row.split_last().unwrap();
                let k: Vec<&str> = k.iter().map(String::as_str).collect();
                if val.is_empty() {
                    self.map.remove(&key(&k)); // empty value = deletion marker
                } else {
                    self.map.insert(key(&k), val.clone());
                }
                n += 1;
            }
        }
        n
    }

    pub fn get(&self, parts: &[&str]) -> Option<&str> {
        self.map.get(&key(parts)).map(String::as_str)
    }

    /// Insert and return the TSV line to append to the backing file.
    pub fn insert(&mut self, parts: &[&str], value: &str) -> String {
        self.map.insert(key(parts), value.to_string());
        let mut fields: Vec<&str> = parts.to_vec();
        fields.push(value);
        tsv::line(&fields)
    }

    /// Remove and return the deletion line to append.
    pub fn remove(&mut self, parts: &[&str]) -> String {
        self.map.remove(&key(parts));
        let mut fields: Vec<&str> = parts.to_vec();
        fields.push("");
        tsv::line(&fields)
    }

    /// Compact representation (one line per live key) for rewriting the file.
    pub fn dump(&self) -> String {
        let mut keys: Vec<&String> = self.map.keys().collect();
        keys.sort();
        let mut out = String::new();
        for k in keys {
            let mut fields: Vec<&str> = k.split('\u{1f}').collect();
            fields.push(&self.map[k]);
            out.push_str(&tsv::line(&fields));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_phrase_stays_with_its_title_language_and_model_after_reload() {
        let first = Scope::new("browser", "First title", Lang::En);
        let second = Scope::new("browser", "Second title", Lang::En);
        let mut s = Store::new();
        let mut file = s.insert(&first.key("gemma", "d", "I'm ready."), "Я готов.");
        file += &s.insert(&second.key("gemma", "d", "I'm ready."), "Я готова.");
        let mut reloaded = Store::new();
        reloaded.load_tsv(&file);
        assert_eq!(reloaded.get(&first.key("gemma", "d", "I'm ready.")), Some("Я готов."));
        assert_eq!(reloaded.get(&second.key("gemma", "d", "I'm ready.")), Some("Я готова."));
        assert_eq!(reloaded.get(&first.key("other-model", "d", "I'm ready.")), None);
        for other in [Scope::new("browser", "First title", Lang::Ja), Scope::new("screen", "First title", Lang::En)] {
            assert_eq!(reloaded.get(&other.key("gemma", "d", "I'm ready.")), None);
        }
    }

    #[test]
    fn unscoped_cache_is_not_reused_and_forgetting_keeps_other_titles() {
        let first = Scope::new("browser", "First", Lang::En);
        let second = Scope::new("browser", "Second", Lang::En);
        let mut s = Store::new();
        s.load_tsv("gemma\td\tWait\tПодожди\n");
        assert_eq!(s.get(&first.key("gemma", "d", "Wait")), None);
        s.insert(&first.key("gemma", "d", "Wait"), "Погоди");
        s.insert(&second.key("gemma", "d", "Wait"), "Стой");
        s.remove(&first.key("gemma", "d", "Wait"));
        let mut reloaded = Store::new();
        reloaded.load_tsv(&s.dump());
        assert_eq!(reloaded.get(&first.key("gemma", "d", "Wait")), None);
        assert_eq!(reloaded.get(&second.key("gemma", "d", "Wait")), Some("Стой"));
    }

    #[test]
    fn old_entries_are_dropped() {
        let sc = Scope::new("browser", "", Lang::Ko);
        let mut s = Store::new();
        s.load_tsv("gemma\td\tWait\tПодожди\n");
        s.insert(&sc.key("gemma", "d", "Wait"), "Стой");
        assert_eq!(s.drop_unscoped(), 1);
        assert_eq!(s.len(), 1);
        assert_eq!(s.get(&sc.key("gemma", "d", "Wait")), Some("Стой"));
    }

    #[test]
    fn append_only_last_wins() {
        let mut s = Store::new();
        let mut file = String::new();
        file += &s.insert(&["hymt", "d", "你好"], "Привет");
        file += &s.insert(&["hymt", "d", "你好"], "Здравствуй");
        file += &s.insert(&["hymt", "d", "再见"], "Пока");
        file += &s.remove(&["hymt", "d", "再见"]);
        let mut r = Store::new();
        r.load_tsv(&file);
        assert_eq!(r.get(&["hymt", "d", "你好"]), Some("Здравствуй"));
        assert_eq!(r.get(&["hymt", "d", "再见"]), None);
        assert_eq!(r.len(), 1);
        let mut r2 = Store::new();
        r2.load_tsv(&r.dump());
        assert_eq!(r2.get(&["hymt", "d", "你好"]), Some("Здравствуй"));
    }
}
