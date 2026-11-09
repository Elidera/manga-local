//! Source languages we translate from, and cheap script-based detection.
//! Manga Local reads Korean (manhwa), Japanese (manga) and Chinese (manhua);
//! English stays for scanlations and old configs.

use crate::text;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Zh,
    En,
    Ko,
    Ja,
}

impl Lang {
    /// "ko" / "ja" / "zh" / "en" (config values). Anything unknown → Chinese.
    pub fn from_code(s: &str) -> Lang {
        let s = s.trim().to_ascii_lowercase();
        if s.starts_with("en") {
            Lang::En
        } else if s.starts_with("ko") {
            Lang::Ko
        } else if s.starts_with("ja") || s.starts_with("jp") {
            Lang::Ja
        } else {
            Lang::Zh
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::Zh => "zh",
            Lang::En => "en",
            Lang::Ko => "ko",
            Lang::Ja => "ja",
        }
    }

    pub fn name_ru(self) -> &'static str {
        match self {
            Lang::Zh => "китайский",
            Lang::En => "английский",
            Lang::Ko => "корейский",
            Lang::Ja => "японский",
        }
    }

    /// «с корейского», «с японского» …
    pub fn from_ru(self) -> &'static str {
        match self {
            Lang::Zh => "с китайского",
            Lang::En => "с английского",
            Lang::Ko => "с корейского",
            Lang::Ja => "с японского",
        }
    }

    /// Name of the language in English (for Hy-MT prompts).
    pub fn name_en(self) -> &'static str {
        match self {
            Lang::Zh => "Chinese",
            Lang::En => "English",
            Lang::Ko => "Korean",
            Lang::Ja => "Japanese",
        }
    }
}

pub fn is_latin_letter(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c as u32, 0xC0..=0x24F)
}

pub fn is_cyrillic(c: char) -> bool {
    matches!(c as u32, 0x400..=0x4FF)
}

pub fn latin_count(s: &str) -> usize {
    s.chars().filter(|&c| is_latin_letter(c)).count()
}

pub fn cyrillic_count(s: &str) -> usize {
    s.chars().filter(|&c| is_cyrillic(c)).count()
}

/// Share of Latin letters among non-space characters.
pub fn latin_ratio(s: &str) -> f32 {
    let total = s.chars().filter(|c| !c.is_whitespace()).count();
    if total == 0 {
        0.0
    } else {
        latin_count(s) as f32 / total as f32
    }
}

/// Guess the language of a piece of text by script. Hangul means Korean and
/// kana means Japanese; ideographs alone are taken as Chinese (a short Japanese
/// line of kanji only, like 大丈夫, is then translated with the Chinese prompt,
/// which Hy-MT handles too).
pub fn detect(s: &str) -> Option<Lang> {
    let han = text::han_count(s);
    let lat = latin_count(s);
    let hangul = text::hangul_count(s);
    let kana = text::kana_count(s);
    if hangul >= 1 && hangul * 3 >= lat + han {
        Some(Lang::Ko)
    } else if kana >= 1 && (kana + han) * 3 >= lat {
        Some(Lang::Ja)
    } else if han >= 1 && han * 3 >= lat {
        Some(Lang::Zh)
    } else if lat >= 2 {
        Some(Lang::En)
    } else {
        None
    }
}

/// Does English text read like a sentence rather than a block of menu labels?
/// Menus are Title Case ("Union Level", "Data Bank"); speech is mostly
/// lower-case words.
pub fn sentence_like_en(s: &str) -> bool {
    let words: Vec<&str> = s
        .split(|c: char| c.is_whitespace() || c == '/' || c == '|')
        .filter(|w| w.chars().next().map_or(false, |c| c.is_alphabetic()))
        .collect();
    let lower = words.iter().filter(|w| w.chars().next().map_or(false, |c| c.is_lowercase())).count();
    let ends = s.trim_end().ends_with(['.', '!', '?', '…', '"']);
    if words.len() <= 2 {
        lower >= 1 || ends
    } else {
        // Game terms are Title Case ("3 Tacet Discords near the Resonance Beacon."):
        // a finished sentence with a few ordinary words is still speech.
        lower * 2 >= words.len() || (ends && lower >= 2)
    }
}

/// Russian text read by the English OCR turns into Latin look-alikes:
/// «Поиск» → "noøcK", «Справка» → "CnpaBKa", «Это того стоило» → "3TO Toro
/// CTOVIJIO", «Пионер» → "nv10Hep", «Магазин» → "CTopa"-like words. Real English
/// almost never has these shapes (rare "iPhone", "McDonald").
fn suspicious_word(w: &str) -> bool {
    let w = w.trim_matches(|c: char| !c.is_alphanumeric());
    let chars: Vec<char> = w.chars().collect();
    let letters = chars.iter().filter(|c| c.is_alphabetic()).count();
    if letters < 2 {
        return false;
    }
    // A capital after a lower-case letter: "noøcK", "naHopaua".
    let mut seen_lower = false;
    for &c in &chars {
        if c.is_lowercase() {
            seen_lower = true;
        } else if c.is_uppercase() && seen_lower {
            return true;
        }
    }
    // Digits inside or before letters: "nv10Hep", "3TO", "56nxcoa" — but not
    // "SOL3", "Lv70", "2nd", "10am".
    if let Some(first_digit) = chars.iter().position(|c| c.is_ascii_digit()) {
        let letters_after = chars[first_digit..].iter().filter(|c| c.is_alphabetic()).count();
        let tail: String = chars[first_digit..].iter().filter(|c| c.is_alphabetic()).collect::<String>().to_lowercase();
        let ordinal = ["st", "nd", "rd", "th", "am", "pm", "s", "x", "d", "k", "m"].contains(&tail.as_str());
        if letters_after >= 2 && !ordinal {
            return true;
        }
    }
    // Two or more capitals then lower-case: "CTopa", "LIHocan" (not "NPCs").
    let caps = chars.iter().take_while(|c| c.is_uppercase()).count();
    if caps >= 2 && chars.len() >= 4 && chars.len() > caps {
        let rest: String = chars[caps..].iter().collect();
        if rest != "s" && rest.chars().all(|c| c.is_lowercase()) {
            return true;
        }
    }
    // Capital look-alikes of Cyrillic letters: "CTOVIJIO" (Ы, Л). "JI" alone is
    // not enough: comics are lettered in capitals ("JI-HO", "JIN-WOO").
    w.contains("IJ")
}

/// Share of words that look like misread Cyrillic (see `suspicious_word`).
pub fn misread_share(s: &str) -> f32 {
    // Hyphenated and underscored words are separate words: "Sub-Stats", "Baishi_Shu".
    let words: Vec<&str> = s
        .split(|c: char| c.is_whitespace() || matches!(c, '-' | '_' | '/' | '—'))
        .filter(|w| w.chars().filter(|c| c.is_alphabetic()).count() >= 2)
        .collect();
    if words.is_empty() {
        return 0.0;
    }
    words.iter().filter(|w| suspicious_word(w)).count() as f32 / words.len() as f32
}

fn looks_misread(s: &str) -> bool {
    let share = misread_share(s);
    let words = s.split_whitespace().count();
    share > 0.0 && (words <= 4 || share >= 0.2)
}

/// Is this OCR result real subtitle text in `lang` (and not noise, icons or another script)?
pub fn is_subtitle(s: &str, lang: Lang) -> bool {
    match lang {
        Lang::Ko => text::hangul_count(s) >= 2 && script_ratio(s, lang) >= 0.4,
        Lang::Ja => text::kana_count(s) + text::han_count(s) >= 2 && script_ratio(s, lang) >= 0.4,
        Lang::Zh => text::han_count(s) >= 2 && text::han_ratio(s) >= 0.4,
        Lang::En => {
            // At least one real word of 3+ letters and mostly letters.
            let has_word = s.split(|c: char| !is_latin_letter(c)).any(|w| w.chars().count() >= 3);
            latin_count(s) >= 4 && latin_ratio(s) >= 0.55 && has_word && sentence_like_en(s) && !looks_misread(s)
        }
    }
}

/// Not text of a comic at all: a web address («asurascans.com»,
/// «https://comic.naver.com/…»), or a line of odd symbols (artwork read as
/// letters: «쬬결출결흐℃e•결결결결», «mnar•aøur rnvnauøu»), or Latin «words»
/// without vowels (Russian text of the browser read as Latin: «mrpbl»).
pub fn is_screen_junk(s: &str) -> bool {
    let low = s.to_lowercase();
    if low.contains("http") || low.contains("www.") || low.contains("://") {
        return true;
    }
    // A domain: letters, a dot, a known ending, no space right after the dot.
    const TLD: &[&str] = &[".com", ".net", ".org", ".ru", ".jp", ".kr", ".io", ".co.", ".me", ".to", ".cc", ".xyz", ".tv", ".info"];
    if low.split_whitespace().any(|w| TLD.iter().any(|t| w.contains(t) && w.len() > t.len() + 1)) {
        return true;
    }
    let chars: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
    if chars.is_empty() {
        return false;
    }
    let odd = chars
        .iter()
        .filter(|&&c| {
            !(c.is_alphanumeric() && (c.is_ascii() || text::is_hangul(c) || text::is_kana(c) || text::is_han(c)) || ".,!?…~-—–'\"“”‘’()「」『』【】、。！？：；:;♡♥☆★♪·*/%&+=".contains(c))
        })
        .count();
    if odd >= 3 || odd >= 2 && odd * 4 > chars.len() {
        return true;
    }
    // One syllable repeated with symbols around: «결결결결» next to «℃».
    if odd >= 1 && chars.windows(4).any(|w| w.iter().all(|&c| c == w[0] && text::is_hangul(c))) {
        return true;
    }
    // Korean with ideographs scattered in it (hanja are written in brackets:
    // 화괴(花魁)): artwork read as letters.
    let mut depth = 0;
    let mut loose_han = 0;
    for c in s.chars() {
        match c {
            '(' | '（' | '[' => depth += 1,
            ')' | '）' | ']' => depth = (depth - 1).max(0),
            c if depth == 0 && text::is_han(c) => loose_han += 1,
            _ => {}
        }
    }
    if loose_han >= 2 && text::hangul_count(s) >= 1 || loose_han >= 1 && (1..=3).contains(&text::hangul_count(s)) {
        return true;
    }
    // Latin words of five and more letters with no vowel at all («NPCs» is fine).
    let words: Vec<&str> = s.split(|c: char| !c.is_ascii_alphabetic()).filter(|w| w.len() >= 5).collect();
    let bare = words.iter().filter(|w| !w.chars().any(|c| "aeiouyAEIOUY".contains(c))).count();
    !words.is_empty() && bare * 2 >= words.len().max(1) && bare >= 1
}

/// Is a single OCR line on a game screen (menu label, button) worth translating?
pub fn is_label(s: &str, lang: Lang) -> bool {
    if is_screen_junk(s) {
        return false;
    }
    match lang {
        // A bubble can hold a single syllable («응», «え»): one letter of the
        // script is enough when the line is mostly that script.
        Lang::Ko => text::hangul_count(s) >= 1 && script_ratio(s, lang) >= 0.4,
        Lang::Ja => text::kana_count(s) + text::han_count(s) >= 1 && script_ratio(s, lang) >= 0.4,
        Lang::Zh => text::has_han(s),
        Lang::En => {
            let has_word = s.split(|c: char| !is_latin_letter(c)).any(|w| w.chars().count() >= 2 && w.chars().any(|c| "aeiouyAEIOUY".contains(c)));
            latin_count(s) >= 2 && latin_ratio(s) >= 0.5 && has_word && !looks_misread(s)
        }
    }
}

/// Common short replies can be genuine even when a bubble has no closed outline.
/// Keep a small allow-list so isolated strokes and arbitrary OCR fragments do
/// not become dialogue. Digits are kept in the key and therefore never match.
pub fn is_short_reply(s: &str, lang: Lang) -> bool {
    let word: String = s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect();
    let replies: &[&str] = match lang {
        Lang::Ja => &["あ", "あっ", "ああ", "え", "えっ", "ええ", "お", "おお", "おい", "うん", "いや", "いいえ", "はい", "ん", "ふん", "へえ", "うわ", "はあ"],
        // Not «아», «어», «오», «야», «네»: alone outside a bubble they are as
        // often bits of the drawing or of the site as real replies.
        Lang::Ko => &["응", "헉", "앗", "아니", "흠", "쳇"],
        Lang::Zh => &["啊", "嗯", "哦", "好", "不", "是", "喂", "唉", "哎", "咦", "哼", "呀", "哈", "对", "對", "不是", "不行"],
        Lang::En => &["no", "oh", "ah", "eh", "ow", "hm", "hmm", "huh", "uh"],
    };
    replies.contains(&word.as_str())
}

/// A small isolated line outside a bubble is usually an OCR fragment. Known
/// replies and emphatic punctuation are retained, including the Japanese «あっ».
pub fn is_loose_fragment(s: &str, lang: Lang, line_h: f32, page_h: f32) -> bool {
    s.chars().filter(|c| c.is_alphanumeric()).count() <= 2
        && !s.contains(['!', '?', '！', '？'])
        && line_h < page_h * 0.035
        && !is_short_reply(s, lang)
}

/// With manga-ocr enabled, short Japanese replies read by the page OCR must
/// survive the artwork filter too. Drawn circles remain excluded.
pub fn is_japanese_outside_text(s: &str) -> bool {
    !s.contains(['〇', '○', '◯'])
        && (text::kana_count(s) + text::han_count(s) >= 4 || is_short_reply(s, Lang::Ja))
}

/// Does a short text look like a sound effect («쾅!», «두근두근», «ドキドキ»,
/// «BANG!»)? Only the shape is judged (a few letters, or a doubled sound,
/// one word); the caller also checks it stands on the artwork, outside every
/// bubble — short replies inside bubbles are speech.
pub fn is_sfx_like(s: &str, lang: Lang) -> bool {
    let letters: Vec<char> = s
        .chars()
        .filter(|&c| match lang {
            Lang::Ko => text::is_hangul(c),
            Lang::Ja => text::is_kana(c) || text::is_han(c),
            Lang::Zh => text::is_han(c),
            Lang::En => is_latin_letter(c),
        })
        .collect();
    let n = letters.len();
    if n == 0 {
        return false;
    }
    let words = s.split_whitespace().filter(|w| w.chars().any(|c| c.is_alphabetic())).count();
    // A doubled sound: «두근두근», «ドキドキ», «ゴゴゴ».
    let doubled = n >= 2 && {
        let half = n / 2;
        (n % 2 == 0 && letters[..half] == letters[half..]) || letters.iter().all(|&c| c == letters[0])
    };
    match lang {
        Lang::En => words == 1 && (n <= 8 || doubled),
        Lang::Ko => words <= 1 && (n <= 3 || doubled),
        Lang::Ja => words <= 1 && (n <= 4 || doubled),
        Lang::Zh => words <= 1 && (n <= 2 || doubled),
    }
}

/// Share of the language's own letters among non-space, non-punctuation
/// characters (punctuation like «……!?» is common in bubbles and is not noise).
pub fn script_ratio(s: &str, lang: Lang) -> f32 {
    let counted: Vec<char> = s.chars().filter(|c| !c.is_whitespace() && !text::is_cjk_punct(*c) && !c.is_ascii_punctuation()).collect();
    if counted.is_empty() {
        return 0.0;
    }
    let own = counted
        .iter()
        .filter(|&&c| match lang {
            Lang::Ko => text::is_hangul(c),
            Lang::Ja => text::is_kana(c) || text::is_han(c),
            Lang::Zh => text::is_han(c),
            Lang::En => is_latin_letter(c),
        })
        .count();
    own as f32 / counted.len() as f32
}

/// The language a page is really in, when it is clearly not the chosen one.
/// The Korean, Japanese and Chinese recognizers read Latin letters too, so an
/// English page read with them comes out as English words with hardly a letter
/// of the chosen script. None: looks like the chosen language, or unclear.
pub fn other_page_language(lines: &[&str], chosen: Lang) -> Option<Lang> {
    if chosen == Lang::En {
        return None;
    }
    let own: usize = lines
        .iter()
        .map(|l| match chosen {
            Lang::Ko => text::hangul_count(l),
            Lang::Ja => text::kana_count(l) + text::han_count(l),
            _ => text::han_count(l),
        })
        .sum();
    let latin: usize = lines.iter().map(|l| latin_count(l)).sum();
    // Lines of real words (three or more of two letters and longer), not a
    // stray «OK» or a number.
    let wordy = lines
        .iter()
        .filter(|l| l.split(|c: char| !is_latin_letter(c) && c != '\'').filter(|w| w.chars().filter(|&c| is_latin_letter(c)).count() >= 2).count() >= 3)
        .count();
    (latin >= 40 && own * 10 < latin && wordy >= 2).then_some(Lang::En)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_replies_survive_both_page_filters_without_a_bubble_outline() {
        // Regression: OCR read «あっ», but F8 discarded it after recognition.
        for s in ["あっ", "「え…」", "うん", "はい", "いいえ"] {
            assert!(is_label(s, Lang::Ja), "{s}");
            assert!(!is_loose_fragment(s, Lang::Ja, 20.0, 1080.0), "{s}");
            assert!(is_japanese_outside_text(s), "{s}");
        }
        for (s, lang) in [("응", Lang::Ko), ("아니…", Lang::Ko), ("嗯", Lang::Zh), ("No…", Lang::En)] {
            assert!(!is_loose_fragment(s, lang, 20.0, 1080.0), "{s}");
        }
    }

    #[test]
    fn short_reply_exception_does_not_admit_ocr_noise() {
        for (s, lang) in [("름", Lang::Ko), ("4수", Lang::Ko), ("一", Lang::Ja), ("〇", Lang::Ja), ("o1", Lang::En)] {
            assert!(is_loose_fragment(s, lang, 20.0, 1080.0), "{s}");
        }
        for s in ["も。〇き朝。", "〇あっ", "一二", ""] {
            assert!(!is_japanese_outside_text(s), "{s}");
        }
        assert!(is_japanese_outside_text("ちょっと待って"));
        assert!(!is_loose_fragment("ドン!", Lang::Ja, 20.0, 1080.0));
        assert!(!is_loose_fragment("一", Lang::Ja, 80.0, 1080.0));
    }

    #[test]
    fn detects() {
        assert_eq!(detect("少侠，此去开封"), Some(Lang::Zh));
        assert_eq!(detect("Rover, wait for me!"), Some(Lang::En));
        assert_eq!(detect("Lv. 30 剑法"), Some(Lang::Zh));
        assert_eq!(detect("30 / 40"), None);
        assert_eq!(Lang::from_code("en-US"), Lang::En);
        assert_eq!(Lang::from_code("zh-Hans-CN"), Lang::Zh);
        assert_eq!(Lang::from_code("ko-KR"), Lang::Ko);
        assert_eq!(Lang::from_code("ja"), Lang::Ja);
        assert_eq!(detect("뭐야, 벌써 왔어?"), Some(Lang::Ko));
        assert_eq!(detect("ちょっと待って！"), Some(Lang::Ja));
        assert_eq!(detect("先輩、遅いですよ"), Some(Lang::Ja));
        assert_eq!(detect("你来了"), Some(Lang::Zh));
    }

    #[test]
    fn korean_and_japanese_bubbles() {
        assert!(is_label("응", Lang::Ko));
        assert!(is_label("뭐야?!", Lang::Ko));
        assert!(is_label("……네.", Lang::Ko));
        assert!(!is_label("123", Lang::Ko));
        assert!(!is_label("Chapter 12", Lang::Ko));
        assert!(!is_label("ちょっと", Lang::Ko));
        assert!(is_label("え？", Lang::Ja));
        assert!(is_label("大丈夫だよ", Lang::Ja));
        assert!(!is_label("뭐야", Lang::Ja));
        assert!(is_subtitle("오늘은 집에 일찍 갈게.", Lang::Ko));
        assert!(!is_subtitle("응", Lang::Ko));
    }

    #[test]
    fn subtitle_filter() {
        assert!(is_subtitle("We should head to Jinzhou before dark.", Lang::En));
        assert!(!is_subtitle("Il |", Lang::En));
        assert!(!is_subtitle("12:45 / 99", Lang::En));
        assert!(!is_subtitle("少侠，此去开封", Lang::En));
        assert!(is_subtitle("少侠，此去开封", Lang::Zh));
        assert!(!is_subtitle("Hello there", Lang::Zh));
        // Menu blocks seen in Wuthering Waves are not subtitles.
        assert!(!is_subtitle("Birthday: Apr 26 Rank 8 SOL3 Phase Union Level Union EXP Expedition Guidebook Synthesis Motorbike", Lang::En));
        assert!(!is_subtitle("Ambient Volume: Interface Volume Off Mute when Game is in the Background", Lang::En));
        assert!(!is_subtitle("Resolution: Global Filter Image Display Mode Apply Recomme", Lang::En));
        assert!(!is_subtitle("KURO: LGAMES", Lang::En));
        assert!(is_subtitle("Jiyan: The frontline won't hold for long. We need reinforcements from Jinzhou by nightfall.", Lang::En));
        assert!(is_subtitle("Thank you.", Lang::En));
        assert!(is_subtitle("Jinhsi: Jinzhou stands because its people refuse to kneel. Remember that.", Lang::En));
    }

    #[test]
    fn misread_russian_is_not_english() {
        for s in [
            "p noøcK",
            "CnpaBKa",
            "HeipoceTb Songtell...",
            "rnaBHas",
            "LIJMH0 naHopaua -.",
            "Hy-MT2 1.8B Q4_K_M — 6blCTpblä YMonqaHmo)",
            ", — 3TO Toro CTOVIJIO.",
            "nv10Hep",
            "CTopa",
            "06Haw6 LW6TMTn6",
        ] {
            assert!(!is_subtitle(s, Lang::En), "{s}");
            assert!(!is_label(s, Lang::En), "{s}");
        }
        assert!(is_subtitle("There's nothing quite like being on duty in a bustling commercial area.", Lang::En));
        assert!(is_subtitle("I bought a new iPhone, and it was worth every coin I had saved.", Lang::En));
        assert!(is_label("Data Bank", Lang::En));
        assert!(is_label("Sub-Stats", Lang::En));
        assert!(is_label("SOL3 Phase", Lang::En));
        assert!(is_label("Lv70 NPCs Rewards", Lang::En));
        assert!(is_subtitle("Meet me on the 2nd floor at 10am, and bring the TV.", Lang::En));
    }

    #[test]
    fn sound_effects() {
        for (s, l) in [("쾅!", Lang::Ko), ("두근두근", Lang::Ko), ("ドキドキ", Lang::Ja), ("ゴゴゴ", Lang::Ja), ("Bang!", Lang::En), ("轰!", Lang::Zh)] {
            assert!(is_sfx_like(s, l), "{s}");
        }
        for (s, l) in [("오늘은 내가 살게.", Lang::Ko), ("ちょっと待って！", Lang::Ja), ("Wait up!", Lang::En)] {
            assert!(!is_sfx_like(s, l), "{s}");
        }
    }

    #[test]
    fn label_filter() {
        assert!(is_label("Inventory", Lang::En));
        assert!(is_label("Lv. 70", Lang::En) == false);
        assert!(is_label("Go", Lang::En));
        // Comic lettering: capitals, Korean names.
        assert!(is_label("JIN-WOO! WAIT UP!", Lang::En));
        assert!(is_label("YOU'RE LATE AGAIN.", Lang::En));
        assert!(is_label("JIHO?!", Lang::En));
        assert!(!is_label("XP", Lang::En));
    }

    #[test]
    fn english_page_read_as_korean() {
        let page = ["I DON'T KNOW WHAT", "YOU'RE TALKING ABOUT.", "WAIT FOR ME AT", "THE STATION, OKAY?"];
        assert_eq!(other_page_language(&page, Lang::Ko), Some(Lang::En));
        assert_eq!(other_page_language(&page, Lang::En), None);
        // A Korean page with an English word or two stays Korean.
        let ko = ["오늘은 내가 살게!", "OK", "정말? 고마워, 형!", "어디로 갈까? NEW YORK?"];
        assert_eq!(other_page_language(&ko, Lang::Ko), None);
        // Too little to tell.
        assert_eq!(other_page_language(&["Next chapter"], Lang::Ja), None);
    }

    #[test]
    fn screen_junk_is_not_text() {
        for junk in ["https://comic.naver.com/webtoon/detail?", "O asurascans.com", "@ O mangaplus.shueisha.co.jp", "쬬결출결흐℃e•결결결결",
                     "mnar•aøur rnvnauøu n Kut+.ønø", "raga mrpbl", "曇구수국구국국흐&되目", "心\"네"] {
            assert!(is_screen_junk(junk), "{junk}");
        }
        for ok in ["기교가 끝도 없이 정밀하게 올라가고 있다.", "HEY! WAIT A MINUTE!", "I'M THANKFUL FOR ALL LIFE IN THIS WORLD.", "ㅋㅋㅋㅋ 진짜?",
                   "今日はいい天気ですね", "대신 값은 치러야지?", "Mr. Kim, 3.5 km away!", "Hmm...", "TSK TSK", "Shh!", "화괴(花魁)가 왔다"] {
            assert!(!is_screen_junk(ok), "{ok}");
        }
    }
}
