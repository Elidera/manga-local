//! manga-bench — run the bundled test lines through a llama-server with exactly
//! the prompts the app uses (manga-core), and print a Markdown report.
//!
//! Usage:
//!   manga-bench --server http://127.0.0.1:8848 --name "Hy-MT2 1.8B" --prompt hymt \
//!               --lines assets/bench/ko_lines.txt --glossary assets/glossary \
//!               [--temp 0.7 --top-p 0.6 --top-k 20 --repeat 1.05 --no-think] [--out report.md]

use serde_json::{json, Value};
use std::time::{Duration, Instant};
use manga_core::glossary::Glossary;
use manga_core::history::History;
use manga_core::prompt::{self, PromptInput, PromptStyle, TextKind};

struct Args {
    server: String,
    name: String,
    prompt: String,
    lines: String,
    glossary: String,
    temp: f32,
    top_p: f32,
    top_k: i32,
    repeat: f32,
    no_think: bool,
    batch: bool,
    /// Batch mode: give each page the last bubbles of the previous one as context.
    context: bool,
    out: Option<String>,
    /// Game title for the prompt; also selects `<profile>_*.tsv` glossaries.
    game: String,
    profile: String,
}

fn parse_args() -> Args {
    let mut a = Args {
        server: "http://127.0.0.1:8848".into(),
        name: "model".into(),
        prompt: "hymt".into(),
        lines: "assets/bench/ko_lines.txt".into(),
        glossary: "assets/glossary".into(),
        temp: 0.7,
        top_p: 0.6,
        top_k: 20,
        repeat: 1.05,
        no_think: false,
        batch: false,
        context: false,
        out: None,
        game: String::new(),
        profile: "browser".into(),
    };
    let v: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < v.len() {
        let val = || v.get(i + 1).cloned().unwrap_or_default();
        match v[i].as_str() {
            "--server" => a.server = val(),
            "--name" => a.name = val(),
            "--prompt" => a.prompt = val(),
            "--lines" => a.lines = val(),
            "--glossary" => a.glossary = val(),
            "--temp" => a.temp = val().parse().unwrap_or(a.temp),
            "--top-p" => a.top_p = val().parse().unwrap_or(a.top_p),
            "--top-k" => a.top_k = val().parse().unwrap_or(a.top_k),
            "--repeat" => a.repeat = val().parse().unwrap_or(a.repeat),
            "--out" => a.out = Some(val()),
            "--game" => a.game = val(),
            "--profile" => a.profile = val(),
            "--no-think" => {
                a.no_think = true;
                i += 1;
                continue;
            }
            "--batch" => {
                a.batch = true;
                i += 1;
                continue;
            }
            "--context" => {
                a.context = true;
                i += 1;
                continue;
            }
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
        i += 2;
    }
    a
}

/// Same rule as the app: `<profile>_*.tsv` only for that profile.
fn load_glossary(dir: &str, profile: &str) -> Glossary {
    const PROFILES: &[&str] = &["browser", "screen", "custom"];
    let mut g = Glossary::new();
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map_or(false, |e| e == "tsv"))
                .filter(|p| {
                    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                    PROFILES.iter().all(|id| *id == profile || !stem.starts_with(&format!("{id}_")))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    for f in files {
        if let Ok(s) = std::fs::read_to_string(&f) {
            let ui = f.file_stem().and_then(|s| s.to_str()).map_or(false, |s| s.contains("game_ui"));
            if ui {
                g.load_tsv_ui(&s);
            } else {
                g.load_tsv(&s);
            }
        }
    }
    g
}

fn chat(agent: &ureq::Agent, a: &Args, messages: &[prompt::Message]) -> Result<String, String> {
    chat_n(agent, a, messages, 256)
}

fn chat_n(agent: &ureq::Agent, a: &Args, messages: &[prompt::Message], max_tokens: u32) -> Result<String, String> {
    chat_schema(agent, a, messages, max_tokens, None)
}

/// As the app: `schema` holds a page batch of an instruct model to its JSON keys.
fn chat_schema(agent: &ureq::Agent, a: &Args, messages: &[prompt::Message], max_tokens: u32, schema: Option<Value>) -> Result<String, String> {
    let msgs: Vec<Value> = messages.iter().map(|m| json!({"role": m.role, "content": m.content})).collect();
    let mut body = json!({
        "messages": msgs, "stream": false, "temperature": a.temp, "top_p": a.top_p,
        "top_k": a.top_k, "repeat_penalty": a.repeat, "max_tokens": max_tokens, "cache_prompt": true,
    });
    if a.no_think {
        body["chat_template_kwargs"] = json!({"enable_thinking": false});
    }
    if let Some(schema) = schema {
        body["response_format"] = json!({"type": "json_schema", "json_schema": {"name": "page", "schema": schema}});
    }
    let resp: Value = agent
        .post(&format!("{}/v1/chat/completions", a.server.trim_end_matches('/')))
        .send_json(body)
        .map_err(|e| e.to_string())?
        .into_json()
        .map_err(|e| e.to_string())?;
    resp["choices"][0]["message"]["content"].as_str().map(str::to_string).ok_or_else(|| format!("bad response: {resp}"))
}

fn md_escape(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

/// Page check: lines in batches of 12 as the bubbles of one page, exactly like
/// the app's F8, with per-bubble fallback when the batch answer cannot be split.
fn run_batch(a: &Args, lines: &[&str], g: &Glossary, agent: &ureq::Agent) {
    let style = PromptStyle::parse(&a.prompt);
    let title = if a.context { "страница (пакетами по 12 пузырей, с предыдущей страницей)" } else { "страница (пакетами по 12 пузырей)" };
    let mut report = format!("## {} — {title}\n\n| Пузырь | Перевод | способ |\n|---|---|---|\n", a.name);
    let (mut batches, mut batch_ok) = (0, 0);
    // The last bubbles of the previous page (original, translation) and how
    // often a context line leaked into an answer.
    let mut prev: Vec<(String, String)> = Vec::new();
    let mut leaks = 0;
    let t_all = Instant::now();
    for chunk in lines.chunks(12) {
        batches += 1;
        let mut terms: Vec<&manga_core::glossary::Term> = Vec::new();
        for l in chunk {
            for t in g.find_in(l, 24) {
                if !terms.iter().any(|x| x.src == t.src) {
                    terms.push(t);
                }
            }
        }
        let ctx: Vec<(&str, &str)> = if a.context { prev.iter().map(|(s, t)| (s.as_str(), t.as_str())).collect() } else { Vec::new() };
        let msgs = prompt::build_batch(style, chunk, &terms, &a.game, TextKind::Dialogue, &ctx);
        let mut page: Vec<(String, String)> = Vec::new();
        let need: usize = chunk.iter().map(|s| s.chars().count() * 3 + 24).sum();
        let json = prompt::batch_is_json(style);
        let tokens = (need as u32 + 64).clamp(128, 3072) + if json { chunk.len() as u32 * 12 } else { 0 };
        let parts = chat_schema(agent, a, &msgs, tokens, json.then(|| prompt::batch_schema(chunk.len())))
            .map(|raw| {
                let p = prompt::parse_batch_any(&raw, chunk.len());
                if p.iter().any(Option::is_none) {
                    eprintln!("batch partially parsed, raw: {raw}");
                }
                p
            })
            .unwrap_or_else(|_| vec![None; chunk.len()]);
        if parts.iter().all(Option::is_some) {
            batch_ok += 1;
        }
        for (l, p) in chunk.iter().zip(parts) {
            if let Some(t) = p.as_ref().and_then(|p| prompt::clean_output(p, l).ok()) {
                let leaked = ctx.iter().any(|(_, c)| c.chars().count() > 6 && t.contains(c));
                if leaked {
                    leaks += 1;
                }
                report.push_str(&format!("| {} | {} | пакет{} |\n", md_escape(l), md_escape(&t), if leaked { " ⚠ повтор контекста" } else { "" }));
                page.push((l.to_string(), t));
                continue;
            }
            let inp = PromptInput {
                source: l,
                kind: TextKind::Dialogue,
                terms: g.find_in(l, 24),
                context_src: String::new(),
                context_pairs: String::new(),
                game: &a.game,
                style_hint: "", speaker: None,
            };
            let t = chat(agent, a, &prompt::build(style, &inp))
                .ok()
                .and_then(|r| prompt::clean_output(&r, l).ok())
                .unwrap_or_else(|| "⚠".into());
            report.push_str(&format!("| {} | {} | по одной |\n", md_escape(l), md_escape(&t)));
            page.push((l.to_string(), t));
        }
        let skip = page.len().saturating_sub(6);
        prev = page.into_iter().skip(skip).collect();
    }
    report.push_str(&format!(
        "\n**Итог:** пакетов {batch_ok}/{batches} разобрано полностью, всего {} мс на {} пузырей{}.\n\n",
        t_all.elapsed().as_millis(),
        lines.len(),
        if a.context { format!(", повторов контекста {leaks}") } else { String::new() }
    ));
    println!("{report}");
    if let Some(p) = &a.out {
        std::fs::write(p, &report).expect("write report");
    }
}

fn main() {
    let a = parse_args();
    let text = std::fs::read_to_string(&a.lines).expect("lines file");
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    let g = load_glossary(&a.glossary, &a.profile);
    let style = PromptStyle::parse(&a.prompt);
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(300)).build();
    if a.batch {
        run_batch(&a, &lines, &g, &agent);
        return;
    }
    let mut hist = History::new(3);

    let mut report = String::new();
    report.push_str(&format!("## {}\n\n| # | Оригинал | Перевод | мс | термины |\n|---|---|---|---|---|\n", a.name));
    let (mut total_ms, mut ok, mut misses, mut failed) = (0u128, 0usize, 0usize, 0usize);
    // Warm-up request so the first line's latency is not dominated by cold start.
    let _ = chat(&agent, &a, &[prompt::Message { role: "user", content: "将以下文本翻译为俄语：你好".into() }]);
    for (i, l) in lines.iter().enumerate() {
        let terms = g.find_in_dialogue(l, 24);
        let inp = PromptInput {
            source: l,
            kind: TextKind::Dialogue,
            terms,
            context_src: hist.source_context(),
            context_pairs: hist.paired_context(),
            game: &a.game,
            style_hint: "", speaker: None,
        };
        let msgs = prompt::build(style, &inp);
        let t0 = Instant::now();
        let res = chat(&agent, &a, &msgs);
        let ms = t0.elapsed().as_millis();
        let (out, miss) = match res.map(|raw| prompt::clean_output(&raw, l).map_err(|e| format!("{e:?}: {raw}"))) {
            Ok(Ok(t)) => {
                ok += 1;
                total_ms += ms;
                let m = g.violations(l, &t).iter().map(|t| t.src.clone()).collect::<Vec<_>>();
                misses += m.len();
                hist.push(l, &t);
                (t, m.join(", "))
            }
            Ok(Err(e)) | Err(e) => {
                failed += 1;
                (format!("⚠ {e}"), String::new())
            }
        };
        report.push_str(&format!("| {} | {} | {} | {ms} | {} |\n", i + 1, md_escape(l), md_escape(&out), miss));
        eprintln!("[{}/{}] {ms} ms  {l}  →  {out}", i + 1, lines.len());
    }
    let avg = if ok > 0 { total_ms / ok as u128 } else { 0 };
    let summary = format!(
        "\n**Итог:** переведено {ok}/{}, ошибок {failed}, среднее время {avg} мс, нарушений глоссария {misses}.\n\n",
        lines.len()
    );
    report.push_str(&summary);
    println!("{report}");
    if let Some(p) = &a.out {
        std::fs::write(p, &report).expect("write report");
    }
    if ok == 0 {
        std::process::exit(1);
    }
}
