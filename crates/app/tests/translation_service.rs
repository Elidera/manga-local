//! Exercise the real translation service on any OS, with a deterministic model
//! and only the Windows-specific dependencies replaced. No model downloads.
#![allow(dead_code)]

#[path = "../src/paths.rs"]
mod paths;
#[path = "../src/translate.rs"]
mod translate;

mod config {
    #[derive(Clone)]
    pub struct ModelPreset {
        pub id: String,
        pub prompt: String,
        pub template: String,
        pub temperature: f32,
        pub max_tokens: u32,
    }

    pub struct Profile {
        pub id: &'static str,
    }

    pub const PROFILES: &[Profile] = &[Profile { id: "browser" }, Profile { id: "screen" }];
}

mod win {
    pub fn local_date_time() -> (String, String) {
        ("2026-11-09".into(), "00:00:00".into())
    }
}

mod llm {
    pub mod client {
        use crate::config::ModelPreset;
        use manga_core::prompt::Message;
        use std::sync::Arc;
        use std::time::Duration;
        use tokio::sync::Notify;

        pub struct LlmClient {
            pub reply: String,
            pub pause: Option<(Arc<Notify>, Arc<Notify>)>,
        }

        impl LlmClient {
            pub async fn chat(
                &self,
                _: &[Message],
                _: &ModelPreset,
                mut partial: impl FnMut(&str),
            ) -> anyhow::Result<String> {
                if let Some((started, resume)) = &self.pause {
                    started.notify_one();
                    resume.notified().await;
                }
                partial(&self.reply);
                Ok(self.reply.clone())
            }

            pub async fn chat_full(
                &self,
                messages: &[Message],
                preset: &ModelPreset,
                _: Option<Duration>,
                _: Option<&serde_json::Value>,
                partial: impl FnMut(&str),
            ) -> anyhow::Result<String> {
                self.chat(messages, preset, partial).await
            }
        }
    }
}

use manga_core::{lang::Lang, prompt::TextKind};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use translate::{Opts, Translator};

struct Fixture {
    paths: paths::Paths,
    tr: Translator,
}

impl Fixture {
    fn new(client: llm::client::LlmClient) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "manga-local-service-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let paths = paths::Paths {
            install: root.join("install"),
            data: root.join("data"),
        };
        std::fs::create_dir_all(paths.user_glossary_dir()).unwrap();
        let tr = Translator::new(paths.clone(), client, 3, "browser");
        Self { paths, tr }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.paths.data.parent().unwrap());
    }
}

fn model() -> config::ModelPreset {
    config::ModelPreset {
        id: "test-model".into(),
        prompt: "instruct".into(),
        template: String::new(),
        temperature: 0.2,
        max_tokens: 256,
    }
}

fn client(reply: &str) -> llm::client::LlmClient {
    llm::client::LlmClient {
        reply: reply.into(),
        pause: None,
    }
}

#[tokio::test]
async fn title_switch_does_not_reuse_exact_or_fuzzy_translations() {
    let f = Fixture::new(client("Я готов идти."));
    let p = model();
    let src = "I am ready to go now.";
    f.tr.ensure_title("browser", "First", Lang::En);
    assert!(
        !f.tr
            .translate(
                &p,
                "First",
                "",
                src,
                TextKind::Dialogue,
                Opts::default(),
                |_| {}
            )
            .await
            .unwrap()
            .cached
    );
    f.tr.ensure_title("browser", "Second", Lang::En);
    assert_eq!(f.tr.lookup(&p.id, src, TextKind::Dialogue), None);
    assert!(
        !f.tr
            .translate(
                &p,
                "Second",
                "",
                src,
                TextKind::Dialogue,
                Opts::default(),
                |_| {}
            )
            .await
            .unwrap()
            .cached
    );
    f.tr.ensure_title("browser", "First", Lang::En);
    assert!(
        f.tr.translate(
            &p,
            "First",
            "",
            src,
            TextKind::Dialogue,
            Opts::default(),
            |_| {}
        )
        .await
        .unwrap()
        .cached
    );
    f.tr.forget(&p.id, src, "Я готов идти.");
    assert_eq!(f.tr.lookup(&p.id, src, TextKind::Dialogue), None);
    let reloaded = Translator::new(f.paths.clone(), client("unused"), 3, "browser");
    reloaded.ensure_title("browser", "Second", Lang::En);
    assert_eq!(
        reloaded.lookup(&p.id, src, TextKind::Dialogue).as_deref(),
        Some("Я готов идти.")
    );
    reloaded.ensure_title("browser", "First", Lang::En);
    assert_eq!(reloaded.lookup(&p.id, src, TextKind::Dialogue), None);
}

#[tokio::test]
async fn batch_results_use_the_same_scoped_cache_as_individual_requests() {
    let f = Fixture::new(client(r#"{"01":"Я готов.","02":"Подожди."}"#));
    let p = model();
    let items = vec!["I am ready.".into(), "Wait here.".into()];
    f.tr.ensure_title("browser", "First", Lang::En);
    let result =
        f.tr.translate_batch(&p, "First", &items, TextKind::Dialogue, &[])
            .await
            .unwrap();
    assert_eq!(
        result,
        vec![Some("Я готов.".into()), Some("Подожди.".into())]
    );
    assert_eq!(
        f.tr.lookup(&p.id, &items[0], TextKind::Dialogue).as_deref(),
        Some("Я готов.")
    );
    f.tr.ensure_title("browser", "Second", Lang::En);
    for src in &items {
        assert_eq!(f.tr.lookup(&p.id, src, TextKind::Dialogue), None);
    }
}

#[tokio::test]
async fn an_in_flight_result_stays_with_the_title_that_requested_it() {
    let started = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let f = Fixture::new(llm::client::LlmClient {
        reply: "Я готов идти.".into(),
        pause: Some((started.clone(), resume.clone())),
    });
    let p = model();
    let src = "I am ready to go now.";
    f.tr.ensure_title("browser", "First", Lang::En);
    let request = f.tr.translate(
        &p,
        "First",
        "",
        src,
        TextKind::Dialogue,
        Opts::default(),
        |_| {},
    );
    let switch = async {
        started.notified().await;
        f.tr.ensure_title("browser", "Second", Lang::En);
        resume.notify_one();
    };
    let (result, ()) = tokio::join!(request, switch);
    result.unwrap();
    assert_eq!(f.tr.lookup(&p.id, src, TextKind::Dialogue), None);
    f.tr.ensure_title("browser", "First", Lang::En);
    assert_eq!(
        f.tr.lookup(&p.id, src, TextKind::Dialogue).as_deref(),
        Some("Я готов идти.")
    );
}

#[tokio::test]
async fn joined_phrase_pieces_are_cached_for_their_original_title() {
    let f = Fixture::new(client("[1] Если ты ещё раз её тронешь, ты труп."));
    let mut p = model();
    p.prompt = "hymt".into();
    let items = vec!["If you touch her again...".into(), "...you're dead.".into()];
    f.tr.ensure_title("browser", "First", Lang::En);
    let parts = f.tr.translate_batch(&p, "First", &items, TextKind::Dialogue, &[]).await.unwrap();
    for (source, part) in items.iter().zip(&parts) {
        assert!(part.as_ref().is_some_and(|s| !s.is_empty()));
        assert_eq!(&f.tr.lookup(&p.id, source, TextKind::Dialogue), part);
    }
    f.tr.ensure_title("browser", "Second", Lang::En);
    for source in &items {
        assert_eq!(f.tr.lookup(&p.id, source, TextKind::Dialogue), None);
    }
}
