//! Noticing character names, to offer them for the title's dictionary.
//!
//! Russian writes names with a capital letter, so the translation shows where
//! a name is. The original spelling is then found in the source bubble by
//! sound: Korean read by Kontsevich's system, Japanese kana by Polivanov's,
//! English letter by letter. A pair seen in two bubbles is offered; names in
//! kanji or Chinese characters cannot be matched this way and are skipped.

use crate::lang::Lang;
use crate::similarity::ratio;
use crate::text::{is_hangul, is_kana};
use std::collections::{HashMap, HashSet};

/// Words that start a Russian sentence with a capital but are not names
/// (addresses like «Хён» are in the glossaries already).
const NOT_NAMES: &[&str] = &[
    "ладно", "да", "нет", "ну", "эй", "слушай", "смотри", "хорошо", "пожалуйста", "прости", "извини", "спасибо", "привет",
    "что", "ох", "ах", "эх", "хм", "боже", "блин", "чёрт", "черт", "ого", "вау", "стой", "подожди", "погоди", "знаешь",
    "кстати", "конечно", "правда", "честно", "короче", "итак", "господин", "госпожа", "барышня", "братик", "сестрёнка",
    "сонбэ", "хён", "нуна", "оппа", "онни", "сэмпай", "сэнсэй", "учитель", "мама", "папа", "брат", "сестра", "дядя", "тётя",
    "вы", "ты", "я", "он", "она", "мы", "они", "это", "так", "вот", "уже", "опять", "снова", "может", "всё", "все",
];

/// Capitalised Russian words that look like names in a translation. A
/// sentence's first word is capitalised anyway: it counts too, unless it is a
/// common word (the match with the original by sound sorts out the rest).
pub fn russian_names(ru: &str) -> Vec<String> {
    let chars: Vec<char> = ru.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !is_cyr(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && (is_cyr(chars[i]) || (chars[i] == '-' && i + 1 < chars.len() && is_cyr(chars[i + 1]))) {
            i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        let capital = word.chars().next().is_some_and(char::is_uppercase);
        let lower = word.to_lowercase();
        if capital && word.chars().count() >= 2 && !NOT_NAMES.contains(&lower.as_str()) && !out.contains(&word) {
            out.push(word);
        }
    }
    out
}

fn is_cyr(c: char) -> bool {
    matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё')
}

/// Korean to Cyrillic, Kontsevich's system (simplified): voiced consonants
/// between vowels, a few common final sounds.
pub fn hangul_to_ru(s: &str) -> String {
    const INIT: [&str; 19] = ["к", "кк", "н", "т", "тт", "р", "м", "п", "пп", "с", "сс", "", "ч", "чч", "чх", "кх", "тх", "пх", "х"];
    const INIT_VOICED: [&str; 19] = ["г", "кк", "н", "д", "тт", "р", "м", "б", "пп", "с", "сс", "", "дж", "чч", "чх", "кх", "тх", "пх", "х"];
    const VOWEL: [&str; 21] = ["а", "э", "я", "йе", "о", "е", "ё", "е", "о", "ва", "вэ", "ве", "ё", "у", "во", "ве", "ви", "ю", "ы", "ый", "и"];
    const FINAL: [&str; 28] = ["", "к", "к", "к", "н", "н", "н", "т", "ль", "к", "м", "ль", "ль", "ль", "п", "ль", "м", "п", "п", "т", "т", "н", "т", "т", "к", "т", "п", "т"];
    let mut out = String::new();
    let mut prev_open = false; // the previous syllable ended in a vowel or a sonorant
    for (n, c) in s.chars().enumerate() {
        if !is_hangul(c) || !('가'..='힣').contains(&c) {
            out.push(c);
            prev_open = false;
            continue;
        }
        let v = c as u32 - 0xAC00;
        let (i, m, f) = ((v / 588) as usize, ((v % 588) / 28) as usize, (v % 28) as usize);
        let init = if n > 0 && prev_open { INIT_VOICED[i] } else { INIT[i] };
        out.push_str(init);
        out.push_str(VOWEL[m]);
        out.push_str(FINAL[f]);
        prev_open = matches!(f, 0 | 4 | 8 | 16 | 21);
    }
    out
}

/// Kana to Cyrillic, Polivanov's system (simplified: small kana and long
/// marks folded in).
pub fn kana_to_ru(s: &str) -> String {
    const ROWS: &[(&str, &str)] = &[
        ("あア", "а"), ("いイ", "и"), ("うウ", "у"), ("えエ", "э"), ("おオ", "о"),
        ("かカ", "ка"), ("きキ", "ки"), ("くク", "ку"), ("けケ", "кэ"), ("こコ", "ко"),
        ("がガ", "га"), ("ぎギ", "ги"), ("ぐグ", "гу"), ("げゲ", "гэ"), ("ごゴ", "го"),
        ("さサ", "са"), ("しシ", "си"), ("すス", "су"), ("せセ", "сэ"), ("そソ", "со"),
        ("ざザ", "дза"), ("じジ", "дзи"), ("ずズ", "дзу"), ("ぜゼ", "дзэ"), ("ぞゾ", "дзо"),
        ("たタ", "та"), ("ちチ", "ти"), ("つツ", "цу"), ("てテ", "тэ"), ("とト", "то"),
        ("だダ", "да"), ("ぢヂ", "дзи"), ("づヅ", "дзу"), ("でデ", "дэ"), ("どド", "до"),
        ("なナ", "на"), ("にニ", "ни"), ("ぬヌ", "ну"), ("ねネ", "нэ"), ("のノ", "но"),
        ("はハ", "ха"), ("ひヒ", "хи"), ("ふフ", "фу"), ("へヘ", "хэ"), ("ほホ", "хо"),
        ("ばバ", "ба"), ("びビ", "би"), ("ぶブ", "бу"), ("べベ", "бэ"), ("ぼボ", "бо"),
        ("ぱパ", "па"), ("ぴピ", "пи"), ("ぷプ", "пу"), ("ぺペ", "пэ"), ("ぽポ", "по"),
        ("まマ", "ма"), ("みミ", "ми"), ("むム", "му"), ("めメ", "мэ"), ("もモ", "мо"),
        ("やヤ", "я"), ("ゆユ", "ю"), ("よヨ", "ё"),
        ("らラ", "ра"), ("りリ", "ри"), ("るル", "ру"), ("れレ", "рэ"), ("ろロ", "ро"),
        ("わワ", "ва"), ("をヲ", "о"), ("んン", "н"),
        ("ゃャ", "я"), ("ゅュ", "ю"), ("ょョ", "ё"), ("ぁァ", "а"), ("ぃィ", "и"), ("ぅゥ", "у"), ("ぇェ", "э"), ("ぉォ", "о"),
    ];
    let mut out = String::new();
    for c in s.chars() {
        if c == 'ー' || c == 'っ' || c == 'ッ' {
            continue;
        }
        match ROWS.iter().find(|(k, _)| k.contains(c)) {
            Some((_, r)) => {
                // Small ya/yu/yo after an i-syllable: «киё» → «кё».
                if matches!(c, 'ゃ' | 'ャ' | 'ゅ' | 'ュ' | 'ょ' | 'ョ') && out.ends_with('и') {
                    out.pop();
                }
                out.push_str(r);
            }
            None => out.push(c),
        }
    }
    out
}

/// English letters to Cyrillic, roughly by sound.
pub fn latin_to_ru(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::new();
    let b: Vec<char> = lower.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let two: String = b[i..(i + 2).min(b.len())].iter().collect();
        let (r, n) = match two.as_str() {
            "sh" => ("ш", 2),
            "ch" => ("ч", 2),
            "kh" => ("х", 2),
            "ts" => ("ц", 2),
            "yu" => ("ю", 2),
            "ya" => ("я", 2),
            "yo" => ("ё", 2),
            "oo" => ("у", 2),
            "ee" => ("и", 2),
            "eo" => ("о", 2),
            "ae" => ("э", 2),
            _ => (
                match b[i] {
                    'a' => "а", 'b' => "б", 'c' => "к", 'd' => "д", 'e' => "е", 'f' => "ф", 'g' => "г", 'h' => "х", 'i' => "и",
                    'j' => "дж", 'k' => "к", 'l' => "л", 'm' => "м", 'n' => "н", 'o' => "о", 'p' => "п", 'q' => "к", 'r' => "р",
                    's' => "с", 't' => "т", 'u' => "у", 'v' => "в", 'w' => "в", 'x' => "кс", 'y' => "й", 'z' => "з", '-' => "-",
                    _ => "",
                },
                1,
            ),
        };
        out.push_str(r);
        i += n;
    }
    out
}

/// A sound skeleton for comparing spellings: voiced and voiceless pairs,
/// е/э/ё, и/й/ы are the same, soft signs and doubles go.
fn skeleton(s: &str) -> String {
    let s = s.to_lowercase().replace("дж", "ч").replace("дз", "з").replace('-', "");
    let mut out = String::new();
    for c in s.chars() {
        let m = match c {
            'б' => 'п',
            'д' => 'т',
            'г' => 'к',
            'ж' => 'ш',
            'з' => 'с',
            'в' => 'ф',
            'э' | 'ё' => 'е',
            'й' | 'ы' => 'и',
            'ь' | 'ъ' => continue,
            c => c,
        };
        if out.ends_with(m) {
            continue;
        }
        out.push(m);
    }
    out
}

/// Where in the source bubble the Russian name `ru` stands: the run of
/// Hangul syllables, kana or Latin letters that sounds most like it.
pub fn match_source(ru: &str, src: &str, lang: Lang) -> Option<String> {
    let target = skeleton(ru);
    if target.chars().count() < 2 {
        return None;
    }
    let mut best: Option<(f32, String)> = None;
    let mut consider = |cand: String, ru_spelling: String| {
        let score = ratio(&skeleton(&ru_spelling), &target);
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            best = Some((score, cand));
        }
    };
    let chars: Vec<char> = src.chars().collect();
    match lang {
        Lang::Ko | Lang::Ja => {
            let in_script = |c: char| if lang == Lang::Ko { is_hangul(c) } else { is_kana(c) && c != '・' };
            let max_len = if lang == Lang::Ko { 4 } else { 8 };
            let mut i = 0;
            while i < chars.len() {
                if !in_script(chars[i]) {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < chars.len() && in_script(chars[i]) {
                    i += 1;
                }
                for a in start..i {
                    for b in a + 1..=(a + max_len).min(i) {
                        let cand: String = chars[a..b].iter().collect();
                        let spelled = if lang == Lang::Ko { hangul_to_ru(&cand) } else { kana_to_ru(&cand) };
                        consider(cand, spelled);
                    }
                }
            }
        }
        Lang::En => {
            for w in src.split(|c: char| !(c.is_ascii_alphabetic() || c == '-')).filter(|w| w.chars().next().is_some_and(|c| c.is_ascii_uppercase())) {
                let w = w.trim_matches('-').to_string();
                consider(w.clone(), latin_to_ru(&w));
            }
        }
        Lang::Zh => return None,
    }
    best.filter(|(s, c)| *s >= 0.75 && c.chars().count() >= 2).map(|(_, c)| c)
}

/// Counts (original, Russian) name pairs over the bubbles of a reading
/// session; a pair seen in two different bubbles is offered once.
#[derive(Default)]
pub struct NameWatch {
    seen: HashMap<(String, String), HashSet<String>>,
    offered: HashSet<String>,
    /// Names settled in this reading: original → Russian form.
    forms: HashMap<String, String>,
}

impl NameWatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Names settled in this reading (seen in two bubbles) whose original is
    /// in `text`: told to the model so a name does not change from page to
    /// page, even before the reader adds it to the title's dictionary.
    pub fn settled_in(&self, text: &str) -> Vec<(String, String)> {
        self.forms.iter().filter(|(o, _)| text.contains(o.as_str())).map(|(o, r)| (o.clone(), r.clone())).collect()
    }

    /// Look at one translated bubble; returns the pairs to offer now.
    /// `known(src)`: already in a dictionary or turned down — skipped.
    pub fn observe(&mut self, src: &str, ru: &str, lang: Lang, known: impl Fn(&str) -> bool) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for name in russian_names(ru) {
            let Some(orig) = match_source(&name, src, lang) else { continue };
            if known(&orig) || self.offered.contains(&orig) {
                continue;
            }
            let bubbles = self.seen.entry((orig.clone(), name.clone())).or_default();
            bubbles.insert(src.to_string());
            if bubbles.len() >= 2 {
                // From now on the model is told this name for the rest of the reading.
                let form = self.forms.entry(orig.clone()).or_insert_with(|| name.clone());
                if name.chars().count() < form.chars().count() {
                    *form = name.clone(); // the shortest form: usually not declined («Минджун», not «Минджуну»)
                }
                self.offered.insert(orig.clone());
                out.push((orig, name));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capitalised_words_in_russian() {
        assert_eq!(russian_names("Джису, что ты будешь делать после уроков?"), ["Джису"]);
        assert_eq!(russian_names("Ладно, пойдём к Минджуну."), ["Минджуну"]);
        assert_eq!(russian_names("Что? Ты уже пришёл?"), Vec::<String>::new());
        assert_eq!(russian_names("Джису улыбнулась."), ["Джису"]);
        assert_eq!(russian_names("Хён, хватит! Сон Джину ждёт."), ["Сон", "Джину"]);
    }

    #[test]
    fn korean_by_sound() {
        assert_eq!(hangul_to_ru("지수"), "чису");
        assert_eq!(hangul_to_ru("민준"), "минджун");
        assert_eq!(match_source("Джису", "지수야, 오늘 수업 끝나고 뭐 해?", Lang::Ko).as_deref(), Some("지수"));
        assert_eq!(match_source("Минджон", "나? 도서관 가려고. 왜, 민준아?", Lang::Ko).as_deref(), Some("민준"));
        assert_eq!(match_source("Москва", "지수야, 오늘 수업 끝나고 뭐 해?", Lang::Ko), None);
    }

    #[test]
    fn japanese_kana_and_english() {
        assert_eq!(kana_to_ru("サクラ"), "сакура");
        assert_eq!(match_source("Сакура", "サクラちゃん、待って！", Lang::Ja).as_deref(), Some("サクラ"));
        assert_eq!(match_source("Джин-у", "Jin-woo! Wait up!", Lang::En).as_deref(), Some("Jin-woo"));
    }

    #[test]
    fn offered_after_two_bubbles() {
        let mut w = NameWatch::new();
        let none = |_: &str| false;
        assert!(w.observe("지수야, 뭐 해?", "Джису, что делаешь?", Lang::Ko, none).is_empty());
        assert!(w.observe("지수야, 뭐 해?", "Джису, что делаешь?", Lang::Ko, none).is_empty(), "the same bubble again does not count");
        assert_eq!(w.observe("지수가 웃었다.", "Джису улыбнулась.", Lang::Ko, none), [("지수".to_string(), "Джису".to_string())]);
        assert!(w.observe("지수야!", "Джису!", Lang::Ko, none).is_empty(), "offered once");
        assert_eq!(w.settled_in("지수가 왔다"), [("지수".to_string(), "Джису".to_string())]);
        assert!(w.settled_in("민준이 왔다").is_empty());
        let mut w2 = NameWatch::new();
        w2.observe("지수야, 뭐 해?", "Джису, что делаешь?", Lang::Ko, |s| s == "지수");
        assert!(w2.observe("지수가 웃었다.", "Джису улыбнулась.", Lang::Ko, |s| s == "지수").is_empty(), "known names are skipped");
    }
}
