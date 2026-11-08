//! Bundled assets must parse and interact sensibly with the core.
use wuxia_core::glossary::Glossary;
use wuxia_core::lang::{self, Lang};
use wuxia_core::prompt::{build, PromptInput, PromptStyle, TextKind};

fn lines(text: &str) -> Vec<&str> {
    text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).collect()
}

fn check(glossary: &str, bench: &str, lang: Lang, min_hits: usize) {
    let mut g = Glossary::new();
    let n = g.load_tsv(glossary);
    assert!(n >= 20, "{lang:?} glossary has {n} terms");
    assert_eq!(n, g.len(), "duplicate source terms in the {lang:?} glossary");
    let ls = lines(bench);
    assert!(ls.len() >= 20, "{lang:?}: only {} test lines", ls.len());
    let mut hits = 0;
    for l in &ls {
        assert_eq!(lang::detect(l), Some(lang), "{l}");
        assert!(lang::is_label(l, lang), "not a bubble: {l}");
        assert_eq!(&wuxia_core::text::normalize_for(l, lang), l, "test line not normalized: {l}");
        let terms = g.find_in(l, 24);
        if !terms.is_empty() {
            hits += 1;
        }
        let inp = PromptInput { source: l, kind: TextKind::Dialogue, terms, context_src: String::new(), context_pairs: String::new(), game: "", style_hint: "", speaker: None };
        for s in [PromptStyle::HyMt, PromptStyle::Instruct] {
            assert!(build(s, &inp).last().unwrap().content.ends_with(l));
        }
    }
    assert!(hits >= min_hits, "{lang:?}: only {hits} test lines use glossary terms");
}

#[test]
fn korean_assets() {
    check(include_str!("../../../assets/glossary/40_manhwa_ko.tsv"), include_str!("../../../assets/bench/ko_lines.txt"), Lang::Ko, 5);
}

#[test]
fn japanese_assets() {
    check(include_str!("../../../assets/glossary/50_manga_ja.tsv"), include_str!("../../../assets/bench/ja_lines.txt"), Lang::Ja, 4);
}

#[test]
fn chinese_assets() {
    // Chinese lines are classical wuxia speech; the idiom glossary is small and optional.
    let mut g = Glossary::new();
    assert!(g.load_tsv(include_str!("../../../assets/glossary/10_chengyu.tsv")) >= 20);
    for l in lines(include_str!("../../../assets/bench/zh_lines.txt")) {
        assert_eq!(lang::detect(l), Some(Lang::Zh), "{l}");
        assert_eq!(&wuxia_core::text::normalize_line(l), l, "bench line not normalized: {l}");
    }
}

#[test]
fn glossaries_do_not_cross_languages() {
    // A Korean term must never match Japanese text and the other way round.
    let mut g = Glossary::new();
    g.load_tsv(include_str!("../../../assets/glossary/40_manhwa_ko.tsv"));
    for l in lines(include_str!("../../../assets/bench/ja_lines.txt")) {
        assert!(g.find_in(l, 24).is_empty(), "{l}");
    }
}
