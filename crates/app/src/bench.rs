//! Model comparison on a corpus in the selected source language.
//!
//! Pause the reading model, then start temporary models one at a time on
//! port+1. After a separate warm-up, translate each line with
//! dialogue context but without cache, and record latency and glossary misses.
//! Output: a Markdown table for side-by-side human review plus a TSV.

use crate::config::Config;
use crate::bench_support::{latency, load_corpus};
use crate::llm::control::pause_model;
use crate::llm::client::LlmClient;
use crate::llm::server;
use crate::state::Shared;
use crate::translate::{Opts, Translator};
use anyhow::{anyhow, ensure, Context, Result};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc::UnboundedSender;
use manga_core::lang::Lang;
use manga_core::prompt::TextKind;

static RUNNING: AtomicBool = AtomicBool::new(false);
pub fn running() -> bool { RUNNING.load(Ordering::SeqCst) }
struct Running;
impl Drop for Running {
    fn drop(&mut self) { RUNNING.store(false, Ordering::SeqCst); }
}

struct Row {
    out: String,
    ms: u128,
    misses: usize,
    success: bool,
}

pub async fn run(shared: Arc<Shared>, supervisor: UnboundedSender<server::ServerCmd>) -> Result<PathBuf> {
    ensure!(RUNNING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok(), "сравнение уже идёт");
    let _running = Running;
    ensure!(!crate::interface::reading_on() && !crate::files::running(), "перед сравнением выключите перевод F8 и остановите перевод главы");
    let cfg = shared.cfg();
    ensure!(cfg.llm.mode != "external", "для сравнения выберите локальный переводчик: программа не переключает модели внешнего сервера");
    ensure!(!cfg.models.is_empty(), "нет моделей для сравнения");
    let port = cfg.llm.port.checked_add(1).ok_or_else(|| anyhow!("нет номера порта для сравнения"))?;
    let lang = cfg.source_lang();
    let corpus = load_corpus(&shared.paths.install, &shared.paths.data, lang).context("не удалось прочитать тестовые реплики")?;
    let (file, lines) = (corpus.path, corpus.lines);
    ensure!(!lines.is_empty(), "набор тестовых реплик пуст");
    let glossary = crate::translate::load_glossary(&shared.paths, &cfg.general.profile, &cfg.general.game_title);
    let progress = |s: String| shared.ui(|u| u.bench_progress = Some(s));
    progress("Сравнение: освобождаю память текущей модели…".into());
    let _pause = pause_model(&supervisor).await?;

    let mut results: Vec<(String, Vec<Row>, String)> = Vec::new();
    for preset in &cfg.models {
        progress(format!("Бенчмарк: {} — запуск…", preset.name));
        let mut bench_cfg: Config = cfg.clone();
        let (client, mut child) = {
            bench_cfg.llm.port = port;
            let client = LlmClient::new(&server::base_url(&bench_cfg), cfg.llm.request_timeout_s);
            let mut st = match server::spawn(&shared.paths, &bench_cfg, preset) {
                Ok(s) => s,
                Err(e) => {
                    results.push((preset.name.clone(), vec![], format!("не запустилась: {e}")));
                    continue;
                }
            };
            let name = preset.name.clone();
            let sh = shared.clone();
            if let Err(e) = server::wait_ready(&client, &mut st, |el, _| {
                sh.ui(|u| u.bench_progress = Some(format!("Бенчмарк: {name} — загрузка ({} с)", el.as_secs())))
            })
            .await
            {
                if st.child.kill().await.is_err() && st.child.try_wait()?.is_none() {
                    return Err(anyhow!("не удалось выгрузить тестовую модель"));
                }
                results.push((preset.name.clone(), vec![], format!("не запустилась: {e}")));
                continue;
            }
            if preset.path.is_empty() {
                if let Some(p) = server::find_downloaded(&shared.paths.models(), &preset.hf) {
                    let mut c = shared.cfg.write().unwrap();
                    if let Some(m) = c.models.iter_mut().find(|m| m.id == preset.id) {
                        m.path = p.to_string_lossy().into_owned();
                    }
                }
                shared.save_config();
            }
            (client, st)
        };

        let tr = Translator::new(shared.paths.clone(), client, cfg.dialogue.context_lines, &cfg.general.profile);
        tr.ensure_title(&cfg.general.profile, &cfg.general.game_title, lang);
        let warmup = match lang {
            Lang::Ko => "안녕하세요.", Lang::Ja => "こんにちは。", Lang::Zh => "你好。", Lang::En => "Hello.",
        };
        progress(format!("Сравнение: {} — прогрев…", preset.name));
        let warm_start = std::time::Instant::now();
        let warmed = tr.translate(preset, &cfg.general.game_title, "", warmup, TextKind::Dialogue,
            Opts { use_cache: false, use_history: false }, |_| {}).await;
        let warmup_ms = warm_start.elapsed().as_millis();
        if let Err(e) = warmed {
            if child.child.kill().await.is_err() && child.child.try_wait()?.is_none() {
                return Err(anyhow!("не удалось выгрузить тестовую модель"));
            }
            results.push((preset.name.clone(), vec![], format!("прогрев не удался: {e}")));
            continue;
        }
        tr.clear_history();
        let mut rows = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            progress(format!("Бенчмарк: {} — {}/{}", preset.name, i + 1, lines.len()));
            let r = tr
                .translate(preset, &cfg.general.game_title, &cfg.dialogue.style_hint, l, TextKind::Dialogue, Opts { use_cache: false, use_history: true }, |_| {})
                .await;
            rows.push(match r {
                Ok(o) => Row { misses: glossary.violations(l, &o.text).len(), out: o.text, ms: o.ms, success: true },
                Err(e) => Row { out: format!("⚠ {e}"), ms: 0, misses: 0, success: false },
            });
        }
        if child.child.kill().await.is_err() && child.child.try_wait()?.is_none() {
            return Err(anyhow!("не удалось выгрузить тестовую модель"));
        }
        let times: Vec<_> = rows.iter().filter(|r| r.success).map(|r| r.ms).collect();
        let timing = latency(&times).map(|t| format!("среднее {} мс, медиана {} мс, 90% ответов до {} мс", t.mean, t.median, t.p90))
            .unwrap_or_else(|| "успешных ответов нет".into());
        let misses: usize = rows.iter().map(|r| r.misses).sum();
        let summary = format!("{timing}; успешно {}/{}, нарушений глоссария: {misses}; прогрев {warmup_ms} мс отдельно", times.len(), lines.len());
        results.push((preset.name.clone(), rows, summary));
    }

    // Markdown report.
    let mut md = String::new();
    let _ = writeln!(md, "# Сравнение моделей — Manga Local\n\nИсточник: `{}`\n", file.display());
    let _ = writeln!(md, "Язык: {}. Тайтл: {}. Модели запускались по одной, текущий переводчик был выгружен. Загрузка и прогрев не входят в замеры. Проверялся перевод отдельных реплик с контекстом, без распознавания и вписывания текста.\n", lang.name_ru(), cfg.general.game_title);
    let _ = writeln!(md, "Движок: `{}`, слоёв на видеокарте: {}, потоки CPU: {}.\n", cfg.llm.server_exe, cfg.llm.gpu_layers, if cfg.llm.threads == 0 { "авто".into() } else { cfg.llm.threads.to_string() });
    for (name, _, summary) in &results {
        let _ = writeln!(md, "* **{name}** — {summary}");
    }
    let _ = writeln!(md, "\nОцените вручную: точность смысла, естественность, интонацию, имена и термины, выдуманные детали.\n");
    for (i, l) in lines.iter().enumerate() {
        let _ = writeln!(md, "### {}. {l}\n", i + 1);
        for (name, rows, _) in &results {
            if let Some(r) = rows.get(i) {
                let _ = writeln!(md, "* *{name}* ({} мс{}): {}", r.ms, if r.misses > 0 { format!(", ⚠ термины: {}", r.misses) } else { String::new() }, r.out);
            }
        }
        md.push('\n');
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let out = shared.paths.data.join(format!("bench-{stamp}.md"));
    std::fs::write(&out, md)?;

    let mut tsv = String::from("line\tmodel\tms\tglossary_misses\ttranslation\tsuccess\n");
    for (i, l) in lines.iter().enumerate() {
        for (name, rows, _) in &results {
            if let Some(r) = rows.get(i) {
                tsv.push_str(&manga_core::tsv::line(&[l, name, &r.ms.to_string(), &r.misses.to_string(), &r.out, if r.success { "true" } else { "false" }]));
            }
        }
    }
    std::fs::write(out.with_extension("tsv"), tsv)?;
    Ok(out)
}
