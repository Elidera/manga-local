//! Prompt construction for the two families of local models we support, and
//! cleanup of their output.
//!
//! * `HyMt` — Tencent Hy-MT / HY-MT translation models. They have no system
//!   prompt and are trained on fixed Chinese instruction templates
//!   (terminology, context, style). We follow those templates closely.
//! * `Instruct` — general instruct models (Qwen3.5, TranslateGemma, …) that
//!   accept a system prompt with detailed literary rules.
//! * A user-defined template can override both (see [`render_custom`]).

use crate::glossary::{Gender, Term};
use crate::lang::{self, Lang};
use crate::text;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptStyle {
    HyMt,
    Instruct,
}

impl PromptStyle {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "hymt" | "hy-mt" | "hunyuan" => PromptStyle::HyMt,
            _ => PromptStyle::Instruct,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextKind {
    /// Spoken line / subtitle: literary register, context matters.
    Dialogue,
    /// Menu label, item name, quest text: concise, consistent.
    Interface,
    /// A character name alone.
    Name,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: &'static str,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct PromptInput<'a> {
    pub source: &'a str,
    pub kind: TextKind,
    pub terms: Vec<&'a Term>,
    /// Previous source lines (oldest first), one per line.
    pub context_src: String,
    /// Previous "source → translation" pairs (oldest first).
    pub context_pairs: String,
    /// Game title shown to instruct models.
    pub game: &'a str,
    /// Extra user style hint (Russian), e.g. «торжественнее», may be empty.
    pub style_hint: &'a str,
    /// Who says the line and their gender, when known (dialogue only): Russian
    /// verbs in the past tense and adjectives about oneself depend on it.
    pub speaker: Option<(&'a str, Gender)>,
}

fn gender_zh(g: Gender) -> &'static str {
    match g {
        Gender::Female => "女性",
        Gender::Male => "男性",
    }
}

fn gender_en(g: Gender) -> &'static str {
    match g {
        Gender::Female => "a woman",
        Gender::Male => "a man",
    }
}

const HYMT_STYLE_DIALOGUE: &str = "漫画对白，口语自然流畅，符合人物性格";
const HYMT_STYLE_INTERFACE: &str = "漫画中的标牌、字幕和拟声词，简洁准确";

fn hymt_terms(terms: &[&Term]) -> String {
    if terms.is_empty() {
        return String::new();
    }
    let mut s = String::from("参考下面的翻译：\n");
    for t in terms {
        s.push_str(&t.src);
        s.push_str(" 翻译成 ");
        s.push_str(&t.dst);
        s.push('\n');
    }
    s.push('\n');
    s
}

fn src_lang(s: &str) -> Lang {
    lang::detect(s).unwrap_or(Lang::Zh)
}

const HYMT_EN_STYLE_DIALOGUE: &str = "natural, expressive spoken Russian of a good manga / webtoon translation";
const HYMT_EN_STYLE_INTERFACE: &str = "short signs, captions and sound effects of a comic";

fn hymt_terms_en(terms: &[&Term]) -> String {
    if terms.is_empty() {
        return String::new();
    }
    let mut s = String::from("Reference the following translations:\n");
    for t in terms {
        s.push_str(&format!("{} translates to {}\n", t.src, t.dst));
    }
    s.push('\n');
    s
}

fn build_hymt_en(inp: &PromptInput) -> Vec<Message> {
    let style = match inp.kind {
        TextKind::Dialogue => HYMT_EN_STYLE_DIALOGUE,
        TextKind::Interface | TextKind::Name => HYMT_EN_STYLE_INTERFACE,
    };
    let mut p = hymt_terms_en(&inp.terms);
    let speaker = inp.speaker.filter(|_| inp.kind == TextKind::Dialogue);
    if inp.kind == TextKind::Dialogue && (!inp.context_src.is_empty() || speaker.is_some()) {
        p.push_str("[Background Information]\n");
        let who = speaker.map(|(name, g)| format!("The speaker of the line below is {name}, {}.", gender_en(g)));
        let ctx: Vec<&str> = who.as_deref().into_iter().chain([inp.context_src.as_str()].into_iter().filter(|c| !c.is_empty())).collect();
        p.push_str(&ctx.join("\n"));
        p.push_str("\n\nPlease translate the following text into Russian, taking the provided background information into account. \
Do not translate the background information. ");
    } else {
        p.push_str("Please translate the following text into Russian. ");
    }
    p.push_str(&format!(
        "Note that the translation style must strictly conform to [{style}]. Only output the translated result without any additional explanation:\n\n"
    ));
    p.push_str(inp.source);
    vec![Message { role: "user", content: p }]
}

fn build_hymt(inp: &PromptInput) -> Vec<Message> {
    // Hy-MT uses the Chinese template only for Chinese sources; Korean,
    // Japanese and English go through the English one.
    if src_lang(inp.source) != Lang::Zh {
        return build_hymt_en(inp);
    }
    let style = match inp.kind {
        TextKind::Dialogue => HYMT_STYLE_DIALOGUE,
        TextKind::Interface | TextKind::Name => HYMT_STYLE_INTERFACE,
    };
    let mut p = hymt_terms(&inp.terms);
    let speaker = inp.speaker.filter(|_| inp.kind == TextKind::Dialogue);
    if inp.kind == TextKind::Dialogue && (!inp.context_src.is_empty() || speaker.is_some()) {
        let who = speaker.map(|(name, g)| format!("说话人：{name}（{}）", gender_zh(g)));
        let ctx: Vec<&str> = who.as_deref().into_iter().chain([inp.context_src.as_str()].into_iter().filter(|c| !c.is_empty())).collect();
        p.push_str(&ctx.join("\n"));
        p.push_str("\n参考上面的信息，把下面的文本翻译成俄语，译文风格必须严格符合：");
        p.push_str(style);
        p.push_str("。注意不需要翻译上文，也不要额外解释：\n");
    } else {
        p.push_str("将以下文本翻译为俄语，译文风格必须严格符合：");
        p.push_str(style);
        p.push_str("。注意只需要输出翻译后的结果，不要额外解释：\n");
    }
    p.push_str(inp.source);
    vec![Message { role: "user", content: p }]
}

/// How names are written in Russian for each source language.
fn names_rule(lang: Lang) -> &'static str {
    match lang {
        Lang::Ko => "Корейские имена транскрибируй по системе Концевича (김민준 — Ким Минджун, 서울 — Сеул).",
        Lang::Ja => "Японские имена транскрибируй по системе Поливанова (さくら — Сакура, 翔太 — Сёта, 東京 — Токио).",
        Lang::Zh => "Китайские имена и топонимы транскрибируй по системе Палладия (李寻 — Ли Сюнь, 开封 — Кайфэн).",
        Lang::En => "Имена транскрибируй по-русски (Jinwoo — Джину, Sakura — Сакура).",
    }
}

/// Forms of address that a Russian reader would not get word for word.
fn address_rule(lang: Lang) -> &'static str {
    match lang {
        Lang::Ko => "Обращения (오빠, 언니, 형, 누나, 선배, 씨, 님) передавай по смыслу и по отношениям героев: по имени, «братик», «сестрёнка», «сонбэ», «господин».",
        Lang::Ja => "Именные суффиксы (-сан, -кун, -тян, -сама, -сэмпай/сэнпай) передавай по смыслу: обычно по имени, «господин», «сэмпай»; не пиши их через дефис к каждому имени.",
        Lang::Zh => "Обращения (师父, 师兄, 前辈, 大人, 哥哥 и т. п.) передавай естественно по контексту: «наставник», «старший брат», «почтенный», «господин».",
        Lang::En => "Обращения передавай естественно по контексту.",
    }
}

fn instruct_system(inp: &PromptInput) -> String {
    let lang = src_lang(inp.source);
    let work = if inp.game.trim().is_empty() { String::new() } else { format!(" Переводишь «{}».", inp.game.trim()) };
    let mut s = format!(
        "Ты — профессиональный переводчик манги, манхвы и маньхуа {from} языка на русский.{work}\n\
Правила:\n\
1. Точно передавай смысл. Ничего не добавляй и не выдумывай.\n\
2. Пиши живым разговорным русским, как в хорошем официальном переводе комикса. Сохраняй характер и интонацию героя: \
грубый говорит грубо, вежливый — вежливо. Реплики в пузырях короткие — перевод тоже должен быть коротким.\n\
3. Идиомы, шутки и игру слов передавай по смыслу, а не дословно.\n\
4. {names}\n\
5. Термины и имена из глоссария используй строго в указанном виде, склоняя по правилам русского языка.\n\
6. {address} Выбирай «ты» или «вы» по отношениям героев.\n\
7. Выведи ТОЛЬКО перевод: без кавычек, пояснений, транскрипции и оригинала.",
        from = lang.from_ru(),
        names = names_rule(lang),
        address = address_rule(lang),
    );
    match inp.kind {
        TextKind::Interface => s.push_str("\n8. Это надпись на картинке (вывеска, табличка, подпись или звук): переводи кратко; не добавляй точку в конце, если её нет в оригинале."),
        TextKind::Name => s.push_str("\n8. Это имя или прозвище героя: дай только русскую форму имени."),
        TextKind::Dialogue => {}
    }
    if !inp.style_hint.trim().is_empty() {
        s.push_str("\nДополнительное пожелание к стилю: ");
        s.push_str(inp.style_hint.trim());
    }
    s
}

fn build_instruct(inp: &PromptInput) -> Vec<Message> {
    let mut u = String::new();
    if !inp.terms.is_empty() {
        u.push_str("Глоссарий:\n");
        for t in &inp.terms {
            u.push_str(&format!("{} — {}", t.src, t.dst));
            if !t.note.is_empty() {
                u.push_str(&format!(" ({})", t.note));
            }
            u.push('\n');
        }
        u.push('\n');
    }
    if let Some((name, g)) = inp.speaker.filter(|_| inp.kind == TextKind::Dialogue) {
        let (who, form) = match g {
            Gender::Female => ("женщина", "женском"),
            Gender::Male => ("мужчина", "мужском"),
        };
        u.push_str(&format!("Говорит {name} ({who}): глаголы прошедшего времени и слова о себе — в {form} роде.\n\n"));
    }
    if inp.kind == TextKind::Dialogue && !inp.context_pairs.is_empty() {
        u.push_str("Предыдущие реплики (не переводи их, это контекст):\n");
        u.push_str(&inp.context_pairs);
        u.push_str("\n\n");
    }
    u.push_str("Переведи на русский:\n");
    u.push_str(inp.source);
    vec![Message { role: "system", content: instruct_system(inp) }, Message { role: "user", content: u }]
}

pub fn build(style: PromptStyle, inp: &PromptInput) -> Vec<Message> {
    match style {
        PromptStyle::HyMt => build_hymt(inp),
        PromptStyle::Instruct => build_instruct(inp),
    }
}

/// Render a user template. Placeholders: `{source}`, `{terms}` (one "src — dst" per
/// line), `{context}` (paired), `{context_src}`, `{game}`, `{style}`.
/// A line `---` separates an optional system part from the user part.
pub fn render_custom(template: &str, inp: &PromptInput) -> Vec<Message> {
    let terms = inp.terms.iter().map(|t| format!("{} — {}", t.src, t.dst)).collect::<Vec<_>>().join("\n");
    let r = |s: &str| {
        s.replace("{source}", inp.source)
            .replace("{terms}", &terms)
            .replace("{context}", &inp.context_pairs)
            .replace("{context_src}", &inp.context_src)
            .replace("{game}", inp.game)
            .replace("{style}", inp.style_hint)
    };
    match template.split_once("\n---\n") {
        Some((sys, user)) => vec![
            Message { role: "system", content: r(sys.trim()) },
            Message { role: "user", content: r(user.trim()) },
        ],
        None => vec![Message { role: "user", content: r(template.trim()) }],
    }
}

/// Separator between items of a batch request (interface mode).
pub const BATCH_SEP: &str = "|||";

/// One request for many texts of one screen: faster than one request per text
/// and keeps names and terms consistent across the page. `Dialogue` = the
/// speech bubbles of one comic page in reading order (each is still translated
/// separately, but the model sees the whole conversation); `Interface` =
/// unrelated short signs and labels.
/// `context`: the last bubbles of the previous page (original, translation) —
/// so the model keeps who speaks to whom, genders and names across pages. They
/// are shown for reference only; the numbered items keep the answer apart.
pub fn build_batch(style: PromptStyle, items: &[&str], terms: &[&Term], game: &str, kind: TextKind, context: &[(&str, &str)]) -> Vec<Message> {
    let n = items.len();
    // Numbered so a dropped or merged item does not shift all the others.
    let joined = items.iter().enumerate().map(|(i, s)| format!("[{}]{s}", i + 1)).collect::<Vec<_>>().join(BATCH_SEP);
    let all: String = items.concat();
    let lang = src_lang(&all);
    let bubbles = kind == TextKind::Dialogue;
    match style {
        PromptStyle::HyMt if lang != Lang::Zh => {
            let mut p = hymt_terms_en(terms);
            if !context.is_empty() {
                p.push_str("[Background Information]\nThe previous speech bubbles of this comic and their Russian translation (for reference only, do not translate or repeat them):\n");
                for (s, t) in context {
                    p.push_str(&format!("{s} → {t}\n"));
                }
                p.push('\n');
            }
            let what = if bubbles {
                format!("{n} speech bubbles of one comic page in reading order")
            } else {
                format!("{n} separate short texts of a comic page (signs, captions, sound effects)")
            };
            let style = if bubbles { HYMT_EN_STYLE_DIALOGUE } else { HYMT_EN_STYLE_INTERFACE };
            p.push_str(&format!(
                "Please accurately translate the following {src} text into Russian. You must retain the exact same number of delimiters in the translation. \
                 The text consists of {what}, separated by \"{BATCH_SEP}\", each prefixed with its number [1], [2]... \
                 Keep the numbers and translate each one separately. Note that the translation style must strictly conform to [{style}]. \
                 Only output the translated result without any additional explanation:\n",
                src = lang.name_en()
            ));
            p.push_str(&joined);
            vec![Message { role: "user", content: p }]
        }
        PromptStyle::HyMt => {
            let mut p = hymt_terms(terms);
            if !context.is_empty() {
                p.push_str("上文（同一部漫画前面的对话及其俄语译文，仅供参考，不要翻译也不要输出）：\n");
                for (s, t) in context {
                    p.push_str(&format!("{s} → {t}\n"));
                }
                p.push('\n');
            }
            let (what, style) = if bubbles { ("同一页漫画中按阅读顺序排列的对话气泡", HYMT_STYLE_DIALOGUE) } else { ("漫画中的独立短文字", HYMT_STYLE_INTERFACE) };
            p.push_str(&format!(
                "将以下文本翻译为俄语。这是{n}条{what}，用“{BATCH_SEP}”分隔，每条前面有编号[1]、[2]……\
                 译文必须保留相同数量的分隔符“{BATCH_SEP}”和原来的编号，且位置不变，每条单独翻译。\
                 译文风格必须严格符合：{style}。注意只需要输出翻译后的结果，不要额外解释：\n"
            ));
            p.push_str(&joined);
            vec![Message { role: "user", content: p }]
        }
        PromptStyle::Instruct => {
            let dummy = PromptInput {
                source: &all,
                kind: if bubbles { TextKind::Dialogue } else { TextKind::Interface },
                terms: vec![],
                context_src: String::new(),
                context_pairs: String::new(),
                game,
                style_hint: "",
                speaker: None,
            };
            let mut sys = instruct_system(&dummy);
            let what = if bubbles { "реплик из пузырей одной страницы, по порядку чтения" } else { "отдельных надписей" };
            sys.push_str(&format!(
                "\n9. На вход подаётся {n} {what}, с номерами [1], [2]…, разделённых «{BATCH_SEP}». \
                 Переведи каждую отдельно (но с учётом общего разговора) и выведи переводы в том же порядке, с теми же номерами, разделяя их тем же «{BATCH_SEP}»."
            ));
            let mut u = String::new();
            if !terms.is_empty() {
                u.push_str("Глоссарий:\n");
                for t in terms {
                    u.push_str(&format!("{} — {}\n", t.src, t.dst));
                }
                u.push('\n');
            }
            if !context.is_empty() {
                u.push_str("Предыдущие реплики (только чтобы понимать разговор; их не переводи и не повторяй):\n");
                for (s, t) in context {
                    u.push_str(&format!("{s} → {t}\n"));
                }
                u.push('\n');
            }
            u.push_str("Переведи на русский:\n");
            u.push_str(&joined);
            vec![Message { role: "system", content: sys }, Message { role: "user", content: u }]
        }
    }
}

/// Split a numbered batch answer. Each slot is `Some` if that item was found:
/// by its `[k]` number, or — when the model dropped the numbers — by position
/// if the count matches exactly.
/// Models sometimes write the separator with extra bars ("|||||"): squeeze any
/// run of 3+ bars into exactly one separator.
fn squeeze_seps(s: &str) -> String {
    let s = s.replace("｜｜｜", BATCH_SEP).replace("| | |", BATCH_SEP);
    let mut out = String::with_capacity(s.len());
    let mut run = 0;
    for c in s.chars() {
        if c == '|' {
            run += 1;
            continue;
        }
        if run >= 3 {
            out.push_str(BATCH_SEP);
        } else {
            out.extend(std::iter::repeat('|').take(run));
        }
        run = 0;
        out.push(c);
    }
    if run >= 3 {
        out.push_str(BATCH_SEP);
    } else {
        out.extend(std::iter::repeat('|').take(run));
    }
    out
}

pub fn parse_batch_indexed(raw: &str, n: usize) -> Vec<Option<String>> {
    let s = squeeze_seps(&strip_think(raw));
    let mut s = s.trim();
    for p in PREFIXES {
        if s.to_lowercase().starts_with(p) {
            s = s[p.len()..].trim_start();
        }
    }
    let parts: Vec<&str> = s.split(BATCH_SEP).map(str::trim).filter(|p| !p.is_empty()).collect();
    let mut out: Vec<Option<String>> = vec![None; n];
    let mut numbered = 0;
    for p in &parts {
        let p2 = p.trim_start_matches(['[', '【', '［']);
        if p2.len() == p.len() {
            continue;
        }
        let digits: String = p2.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            continue;
        }
        let rest = p2[digits.len()..].trim_start_matches([']', '】', '］', '.', ')', ' ']).trim();
        if let Ok(k) = digits.parse::<usize>() {
            if (1..=n).contains(&k) && !rest.is_empty() && out[k - 1].is_none() {
                out[k - 1] = Some(rest.to_string());
                numbered += 1;
            }
        }
    }
    if numbered == 0 && parts.len() == n {
        return parts.into_iter().map(|p| Some(p.to_string())).collect();
    }
    out
}

/// Split a batch answer back into items. `None` if the count does not match.
pub fn parse_batch(raw: &str, n: usize) -> Option<Vec<String>> {
    let s = strip_think(raw).replace("｜｜｜", BATCH_SEP).replace("| | |", BATCH_SEP);
    let mut s = s.trim();
    for p in PREFIXES {
        if s.to_lowercase().starts_with(p) {
            s = s[p.len()..].trim_start();
        }
    }
    let parts: Vec<String> = s.split(BATCH_SEP).map(|p| p.trim().to_string()).collect();
    (parts.len() == n && parts.iter().all(|p| !p.is_empty())).then_some(parts)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reject {
    Empty,
    /// Output is still mostly Chinese — the model did not translate.
    NotTranslated,
    /// Output is wildly longer than any sane translation (rambling / hallucination).
    TooLong,
    /// The model talks about the text instead of translating it («Это не похоже
    /// на текст…», «(Нет перевода)»): the source was OCR noise or a web address.
    Refusal,
}

/// Does the answer talk about the task instead of translating?
pub fn is_refusal(s: &str) -> bool {
    let low = s.to_lowercase();
    const SIGNS: &[&str] = &[
        "не похоже на текст",
        "не является осмысленн",
        "не удалось распознать",
        "не удалось перевести",
        "нет текста для перевода",
        "нет перевода",
        "невозможно перевести",
        "не могу перевести",
        "предоставьте текст",
        "предоставьте читаемый",
        "предоставьте мангу",
        "пожалуйста, предоставьте",
        "не текст для перевода",
        "это ссылка",
        "просматривать внешние",
        "готов работать как",
        "как профессиональный переводчик",
    ];
    SIGNS.iter().any(|w| low.contains(w)) || low.starts_with('(') && low.ends_with(')') && low.contains("перевод")
}

fn strip_think(s: &str) -> String {
    let mut out = s.to_string();
    while let Some(a) = out.find("<think>") {
        match out[a..].find("</think>") {
            Some(b) => out.replace_range(a..a + b + "</think>".len(), ""),
            None => {
                out.truncate(a);
                break;
            }
        }
    }
    out
}

const PREFIXES: &[&str] = &["перевод:", "перевод на русский:", "translation:", "russian:", "译文：", "译文:", "русский:", "ответ:"];

/// Turn raw model output into a displayable translation, or reject it.
fn share(s: &str, count: fn(&str) -> usize) -> f32 {
    let total = s.chars().filter(|c| !c.is_whitespace()).count();
    if total == 0 {
        0.0
    } else {
        count(s) as f32 / total as f32
    }
}

pub fn clean_output(raw: &str, source: &str) -> Result<String, Reject> {
    let mut s = strip_think(raw).trim().to_string();
    // Remove label prefixes.
    loop {
        let low = s.to_lowercase();
        match PREFIXES.iter().find(|p| low.starts_with(*p)) {
            Some(p) => s = s[p.len()..].trim_start().to_string(),
            None => break,
        }
    }
    // Some models echo the source on its own line.
    if let Some(rest) = s.strip_prefix(source) {
        let rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '→' || c == '-' || c == '—');
        if !rest.is_empty() {
            s = rest.to_string();
        }
    }
    // Keep only the first paragraph for short sources (models sometimes add notes).
    if source.chars().count() < 120 {
        if let Some(i) = s.find("\n\n") {
            s.truncate(i);
        }
    }
    let s = s.trim();
    // Strip wrapping quotes the source did not have.
    let src_quoted = source.starts_with(['“', '「', '"', '《']);
    let s = if !src_quoted {
        let pairs = [('«', '»'), ('"', '"'), ('“', '”'), ('„', '“')];
        let mut t = s;
        for (o, c) in pairs {
            if t.starts_with(o) && t.ends_with(c) && t.chars().count() >= 2 {
                let inner = &t[o.len_utf8()..t.len() - c.len_utf8()];
                if !inner.contains(o) {
                    t = inner;
                }
            }
        }
        t
    } else {
        s
    };
    if s.is_empty() {
        return Err(Reject::Empty);
    }
    if is_refusal(s) {
        return Err(Reject::Refusal);
    }
    if text::han_ratio(s) > 0.3 || share(s, text::hangul_count) > 0.3 || share(s, text::kana_count) > 0.3 {
        return Err(Reject::NotTranslated);
    }
    // English source: the answer must be mostly Cyrillic (names may stay Latin).
    let lat = lang::latin_count(s);
    if lat >= 6 && lang::cyrillic_count(s) * 2 < lat && lang::detect(source) == Some(Lang::En) {
        return Err(Reject::NotTranslated);
    }
    // Russian is ~2–4× longer in chars than Chinese; allow generous slack.
    let limit = 40 + source.chars().count() * 8;
    if s.chars().count() > limit {
        return Err(Reject::TooLong);
    }
    Ok(text::tidy_russian(s))
}

#[cfg(test)]
mod tests {
    #[test]
    fn speaker_gender_reaches_every_prompt() {
        let mk = |src: &'static str| PromptInput {
            source: src,
            kind: TextKind::Dialogue,
            terms: vec![],
            context_src: String::new(),
            context_pairs: String::new(),
            game: "WuWa",
            style_hint: "",
            speaker: Some(("Yinlin", Gender::Female)),
        };
        let en = build(PromptStyle::HyMt, &mk("I came back for you."));
        assert!(en[0].content.contains("The speaker of the line below is Yinlin, a woman."), "{}", en[0].content);
        assert!(en[0].content.ends_with("I came back for you."));
        let zh = build(PromptStyle::HyMt, &mk("我回来找你了。"));
        assert!(zh[0].content.starts_with("说话人：Yinlin（女性）\n参考上面的信息"), "{}", zh[0].content);
        let ins = build(PromptStyle::Instruct, &mk("I came back for you."));
        assert!(ins[1].content.contains("в женском роде"));
        let mut ui = mk("Back");
        ui.kind = TextKind::Interface;
        assert!(!build(PromptStyle::HyMt, &ui)[0].content.contains("speaker"));
    }

    use super::*;

    fn term(s: &str, d: &str) -> Term {
        Term { src: s.into(), dst: d.into(), note: String::new() }
    }

    #[test]
    fn hymt_with_terms_and_context() {
        let t = term("开封", "Кайфэн");
        let inp = PromptInput {
            source: "我们去开封。",
            kind: TextKind::Dialogue,
            terms: vec![&t],
            context_src: "少侠，你醒了？".into(),
            context_pairs: String::new(),
            game: "Where Winds Meet",
            style_hint: "", speaker: None,
        };
        let m = build(PromptStyle::HyMt, &inp);
        assert_eq!(m.len(), 1);
        let c = &m[0].content;
        assert!(c.starts_with("参考下面的翻译：\n开封 翻译成 Кайфэн\n"));
        assert!(c.contains("少侠，你醒了？\n参考上面的信息"));
        assert!(c.ends_with("\n我们去开封。"));
    }

    #[test]
    fn english_source() {
        let t = term("Rover", "Скиталец");
        let inp = PromptInput {
            source: "Rover, we need to reach Jinzhou.",
            kind: TextKind::Dialogue,
            terms: vec![&t],
            context_src: "Where are we?".into(),
            context_pairs: "Where are we? → Где мы?".into(),
            game: "Wuthering Waves",
            style_hint: "", speaker: None,
        };
        let m = build(PromptStyle::HyMt, &inp);
        let c = &m[0].content;
        assert!(c.starts_with("Reference the following translations:\nRover translates to Скиталец\n"));
        assert!(c.contains("[Background Information]\nWhere are we?"));
        assert!(c.ends_with("\n\nRover, we need to reach Jinzhou."));
        let m = build(PromptStyle::Instruct, &inp);
        assert!(m[0].content.contains("с английского"));
        let b = build_batch(PromptStyle::HyMt, &["Inventory", "Quests"], &[], "G", TextKind::Interface, &[]);
        assert!(b[0].content.contains("same number of delimiters"));
        // Untranslated English answer is rejected; Russian with a Latin name passes.
        assert_eq!(clean_output("Rover, we need to go.", "Rover, we need to go."), Err(Reject::NotTranslated));
        assert_eq!(clean_output("Rover, нам нужно идти.", "Rover, we need to go.").unwrap(), "Rover, нам нужно идти.");
    }

    #[test]
    fn instruct_has_system() {
        let inp = PromptInput {
            source: "确定",
            kind: TextKind::Interface,
            terms: vec![],
            context_src: String::new(),
            context_pairs: "a → b".into(),
            game: "WWM",
            style_hint: "", speaker: None,
        };
        let m = build(PromptStyle::Instruct, &inp);
        assert_eq!(m[0].role, "system");
        assert!(m[0].content.contains("надпись на картинке"));
        assert!(!m[1].content.contains("Предыдущие")); // no context for UI
    }

    #[test]
    fn previous_page_as_context() {
        let ctx = [("선배, 늦었어요!", "Сонбэ, вы опоздали!")];
        let b = build_batch(PromptStyle::HyMt, &["미안해.", "가자."], &[], "", TextKind::Dialogue, &ctx);
        let p = &b[0].content;
        assert!(p.contains("선배, 늦었어요! → Сонбэ, вы опоздали!"));
        // The context comes before the items to translate; the items end the prompt.
        assert!(p.find("Сонбэ").unwrap() < p.find("[1]미안해.").unwrap());
        assert!(p.ends_with("[2]가자."));
        let i = build_batch(PromptStyle::Instruct, &["미안해."], &[], "", TextKind::Dialogue, &ctx);
        assert!(i[1].content.contains("Предыдущие реплики") && i[1].content.ends_with("[1]미안해."));
        let none = build_batch(PromptStyle::HyMt, &["미안해."], &[], "", TextKind::Dialogue, &[]);
        assert!(!none[0].content.contains("Background"));
    }

    #[test]
    fn manga_prompts_for_korean_and_japanese() {
        let inp = |src| PromptInput {
            source: src,
            kind: TextKind::Dialogue,
            terms: vec![],
            context_src: String::new(),
            context_pairs: String::new(),
            game: "",
            style_hint: "",
            speaker: None,
        };
        let ko = build(PromptStyle::HyMt, &inp("뭐야, 벌써 왔어?"));
        assert!(ko[0].content.starts_with("Please translate the following text into Russian."), "{}", ko[0].content);
        assert!(ko[0].content.contains("manga / webtoon"));
        let ja = build(PromptStyle::Instruct, &inp("ちょっと待って！"));
        assert!(ja[0].content.contains("с японского"));
        assert!(ja[0].content.contains("Поливанова"));
        assert!(!ja[0].content.contains("«»"), "empty title must not be quoted");
        let b = build_batch(PromptStyle::HyMt, &["뭐야?", "왔어."], &[], "", TextKind::Dialogue, &[]);
        assert!(b[0].content.contains("2 speech bubbles of one comic page"), "{}", b[0].content);
        assert!(b[0].content.contains("Korean text"));
        assert!(b[0].content.ends_with("[1]뭐야?|||[2]왔어."));
        assert_eq!(clean_output("뭐야, 벌써 왔어?", "뭐야, 벌써 왔어?"), Err(Reject::NotTranslated));
        assert_eq!(clean_output("ちょっと待って！", "ちょっと待って！"), Err(Reject::NotTranslated));
        assert_eq!(clean_output("Что, уже пришёл?", "뭐야, 벌써 왔어?").unwrap(), "Что, уже пришёл?");
    }

    #[test]
    fn custom_template() {
        let inp = PromptInput {
            source: "你好",
            kind: TextKind::Dialogue,
            terms: vec![],
            context_src: String::new(),
            context_pairs: String::new(),
            game: "G",
            style_hint: "", speaker: None,
        };
        let m = render_custom("Sys {game}\n---\nTranslate: {source}", &inp);
        assert_eq!(m[0].content, "Sys G");
        assert_eq!(m[1].content, "Translate: 你好");
    }

    #[test]
    fn batch_roundtrip() {
        let t = term("背包", "Инвентарь");
        let m = build_batch(PromptStyle::HyMt, &["背包", "任务", "设置"], &[&t], "G", TextKind::Interface, &[]);
        assert!(m[0].content.contains("[1]背包|||[2]任务|||[3]设置"));
        // Model dropped item 2 and added a leading separator: the others survive.
        let r = parse_batch_indexed("|||[1]Инвентарь|||[3] Настройки", 3);
        assert_eq!(r, vec![Some("Инвентарь".to_string()), None, Some("Настройки".to_string())]);
        // Numbers dropped but the count matches: by position.
        assert_eq!(parse_batch_indexed("А|||Б", 2), vec![Some("А".to_string()), Some("Б".to_string())]);
        assert_eq!(parse_batch_indexed("А|||Б", 3), vec![None, None, None]);
        // Extra bars in the separator (seen with Hy-MT2 on English labels).
        assert_eq!(
            parse_batch_indexed("[1]Инвентарь|||||[2]Почта||||[3]Отмена", 3),
            vec![Some("Инвентарь".to_string()), Some("Почта".to_string()), Some("Отмена".to_string())]
        );
        assert!(m[0].content.starts_with("参考下面的翻译"));
        let m = build_batch(PromptStyle::Instruct, &["背包", "任务"], &[], "G", TextKind::Interface, &[]);
        assert_eq!(m[0].role, "system");
        assert_eq!(parse_batch("Инвентарь ||| Задания|||Настройки", 3).unwrap(), vec!["Инвентарь", "Задания", "Настройки"]);
        assert_eq!(parse_batch("Инвентарь｜｜｜Задания", 2).unwrap().len(), 2);
        assert!(parse_batch("Инвентарь|||Задания", 3).is_none());
        assert!(parse_batch("Инвентарь||||||", 3).is_none());
    }

    #[test]
    fn cleanup() {
        assert_eq!(clean_output("<think>hmm</think>\nПеревод: «Идём в Кайфэн.»", "我们去开封。").unwrap(), "Идём в Кайфэн.");
        assert_eq!(clean_output("我们去开封。\n→ Идём", "我们去开封。").unwrap(), "Идём");
        assert_eq!(clean_output("我们去开封", "我们去开封"), Err(Reject::NotTranslated));
        assert_eq!(clean_output("  ", "x"), Err(Reject::Empty));
        // Noise read from the screen: the model explains instead of translating.
        assert_eq!(clean_output("Это не похоже на текст, который можно перевести. Пожалуйста, предоставьте читаемый текст.", "raga mrpbl"), Err(Reject::Refusal));
        assert_eq!(clean_output("(Нет перевода)", "Vihc\""), Err(Reject::Refusal));
        assert_eq!(clean_output("(Невозможно перевести, так как предоставленный текст не является осмысленным корейским языком.)", "결결결"), Err(Reject::Refusal));
        assert_eq!(clean_output("Нет, я не пойду.", "No, I won't go."), Ok("Нет, я не пойду.".into()));
        assert_eq!(clean_output("Да.\n\nПримечание: ...", "是。").unwrap(), "Да.");
    }
}
