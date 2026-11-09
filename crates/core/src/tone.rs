//! The intonation of a bubble — a shout, a whisper, a stutter, a phrase cut
//! off — which a translation model likes to "fix": a shout becomes a calm
//! sentence, «W-what…» becomes «Что?». Found here from the text and the size
//! of its letters, told to the model with the bubble, and put back afterwards
//! by plain rules (`keep`) when the model lost it anyway.

use crate::text;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tone {
    /// «STOP!!», big letters with «!»: lettered in capitals.
    pub shout: bool,
    /// Small letters: said quietly.
    pub whisper: bool,
    /// «W-what», «그, 그게», «あ、あの».
    pub stutter: bool,
    /// Ends with «…» or «—»: the phrase is cut off.
    pub trailing: bool,
}

impl Tone {
    pub fn is_plain(&self) -> bool {
        *self == Tone::default()
    }

    /// In words, for the model: «крик, заикание». None for a plain bubble.
    pub fn label(&self) -> Option<String> {
        let mut v = Vec::new();
        if self.shout {
            v.push("крик");
        }
        if self.whisper {
            v.push("шёпот");
        }
        if self.stutter {
            v.push("заикание");
        }
        if self.trailing {
            v.push("обрыв");
        }
        (!v.is_empty()).then(|| v.join(", "))
    }
}

/// The punctuation a phrase ends with, normalised: «...» → «…», «！» → «!».
fn ending(s: &str) -> String {
    let t = s.trim_end();
    let tail: String = t.chars().rev().take_while(|c| "!?.…‥~！？。、—–-".contains(*c)).collect::<Vec<_>>().into_iter().rev().collect();
    tail.replace("...", "…").replace("..", "…").replace('‥', "…").replace('！', "!").replace('？', "?").replace('。', ".")
}

/// Does the phrase start with a stutter?
fn starts_with_stutter(s: &str) -> bool {
    let s = s.trim_start_matches(|c: char| !c.is_alphanumeric());
    let chars: Vec<char> = s.chars().collect();
    // Letters, a hyphen, the same letters again: «W-what», «I-I», «Н-нет».
    if let Some(h) = chars.iter().position(|&c| c == '-') {
        if (1..=3).contains(&h) && chars[..h].iter().all(|c| c.is_alphabetic()) {
            let pre: String = chars[..h].iter().collect::<String>().to_lowercase();
            let post: String = chars[h + 1..].iter().take(h).collect::<String>().to_lowercase();
            if pre == post {
                return true;
            }
        }
    }
    // One or two syllables or kana, a comma or dots, the same again: «그, 그게», «あ、あの».
    let first = match chars.first() {
        Some(&c) if text::is_hangul(c) || text::is_kana(c) || text::is_han(c) => c,
        _ => return false,
    };
    let mut i = 1;
    if chars.get(1).is_some_and(|&c| c == first) && chars.get(2).is_some_and(|c| ",、…. ".contains(*c)) {
        i = 2; // «저저, 저기»
    }
    let sep = chars[i..].iter().take_while(|c| ",、…. ‥".contains(**c)).count();
    if sep == 0 {
        return false;
    }
    let next = chars.get(i + sep).copied();
    // Japanese: a single kana and a comma is already a stutter («べ、別に»).
    next == Some(first) || text::is_kana(first) && i == 1 && chars.get(1) == Some(&'、')
}

/// The intonation of a bubble. `raw`: its text as read (before any change of
/// case); `size`: the height of its letters against the usual ones on the
/// page (1.0 = as most bubbles, 0 = unknown).
pub fn detect(raw: &str, size: f32) -> Tone {
    let end = ending(raw);
    let bangs = end.chars().filter(|&c| c == '!').count();
    let big = size >= 1.45;
    let small = size > 0.0 && size <= 0.7;
    Tone {
        shout: bangs >= 2 || bangs >= 1 && big,
        whisper: small && bangs == 0,
        stutter: starts_with_stutter(raw),
        trailing: (end.ends_with('…') || end.ends_with('—') || end.ends_with('–')) && !end.contains('!') && !end.contains('?'),
    }
}

/// Put the intonation back into a translation the model smoothed over:
/// the ending «!», «!!», «?!», «…» of the original, the stutter, and a shout
/// in capitals.
pub fn keep(src: &str, dst: &str, t: Tone) -> String {
    let mut d = dst.trim().to_string();
    if d.is_empty() {
        return d;
    }
    // A stutter: «Нет, я не…» → «Н-нет, я не…».
    if t.stutter && !starts_with_stutter(&d) {
        let lead: String = d.chars().take_while(|c| !c.is_alphabetic()).collect();
        let rest = &d[lead.len()..];
        if let Some(first) = rest.chars().next().filter(|c| c.is_alphabetic()) {
            let tail = &rest[first.len_utf8()..];
            let lower: String = first.to_lowercase().collect();
            d = format!("{lead}{first}-{lower}{tail}");
        }
    }
    let src_end = ending(src);
    let dst_end = ending(&d);
    let src_bangs = src_end.chars().filter(|&c| c == '!').count();
    let dst_bangs = dst_end.chars().filter(|&c| c == '!').count();
    if src_bangs > 0 && dst_bangs < src_bangs.min(3) {
        // «Стой.» → «Стой!!»; «Правда?» for «Really?!» → «Правда?!».
        while d.ends_with('.') || d.ends_with('。') {
            d.pop();
        }
        d.push_str(&"!".repeat(src_bangs.min(3) - dst_bangs));
    } else if src_end.contains('…') && !src_end.contains(['!', '?']) && !dst_end.contains('…') && !dst_end.contains(['!', '?']) {
        // A phrase cut off stays cut off: «Я не…», not «Я не.».
        while d.ends_with('.') {
            d.pop();
        }
        d.push('…');
    }
    if t.shout {
        d = d.to_uppercase();
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_intonation() {
        assert!(detect("STOP RIGHT THERE!!", 1.0).shout);
        assert!(detect("Stop!", 1.6).shout);
        assert!(!detect("Stop!", 1.0).shout);
        assert!(detect("I'm fine...", 0.6).whisper);
        assert!(detect("W-what was that?!", 1.0).stutter);
        assert!(detect("I-I didn't mean it", 1.0).stutter);
        assert!(detect("그, 그게 아니라…", 1.0).stutter);
        assert!(detect("あ、あの…", 1.0).stutter);
        assert!(detect("べ、別にあんたのためじゃない！", 1.0).stutter);
        assert!(!detect("Well, maybe.", 1.0).stutter);
        assert!(!detect("Jin-woo! Wait up!", 1.0).stutter);
        assert!(detect("그게 아니라...", 1.0).trailing);
        assert!(!detect("뭐?!", 1.0).trailing);
        assert!(detect("Hello.", 1.0).is_plain());
    }

    #[test]
    fn puts_it_back() {
        let shout = detect("STOP RIGHT THERE!!", 1.0);
        assert_eq!(keep("STOP RIGHT THERE!!", "Стой на месте.", shout), "СТОЙ НА МЕСТЕ!!");
        let st = detect("N-no, I didn't...", 1.0);
        assert_eq!(keep("N-no, I didn't...", "Нет, я не.", st), "Н-нет, я не…");
        // Already right: nothing changes.
        assert_eq!(keep("W-what?!", "Ч-что?!", detect("W-what?!", 1.0)), "Ч-что?!");
        assert_eq!(keep("Really?!", "Правда?", Tone::default()), "Правда?!");
        assert_eq!(keep("Hello.", "Привет.", Tone::default()), "Привет.");
        assert_eq!(keep("그게 아니라…", "Дело не в этом.", detect("그게 아니라…", 1.0)), "Дело не в этом…");
        // The model answered a cut phrase with a question: left alone.
        assert_eq!(keep("그게…", "Что?", Tone::default()), "Что?");
    }
}
