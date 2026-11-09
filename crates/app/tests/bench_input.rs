#[path = "../src/bench_support.rs"]
mod bench_support;

use bench_support::{latency, load_corpus};
use manga_core::lang::Lang;

#[test]
fn comparison_loads_the_selected_language_and_respects_custom_corpus() {
    let root = std::env::temp_dir().join(format!("manga-bench-corpus-{}", std::process::id()));
    let install = root.join("install");
    let data = root.join("data");
    std::fs::create_dir_all(install.join("bench")).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(
        install.join("bench/en_lines.txt"),
        "# English\nWait here.\n\n",
    )
    .unwrap();
    std::fs::write(install.join("bench/ko_lines.txt"), "기다려.\n").unwrap();
    let en = load_corpus(&install, &data, Lang::En).unwrap();
    assert_eq!(en.lines, vec!["Wait here."]);
    assert_eq!(
        load_corpus(&install, &data, Lang::Ko).unwrap().lines,
        vec!["기다려."]
    );
    std::fs::write(
        data.join("bench.txt"),
        "\u{feff}# Own scene\nHello!\n  Stay.  \n",
    )
    .unwrap();
    let custom = load_corpus(&install, &data, Lang::En).unwrap();
    assert_eq!(custom.path, data.join("bench.txt"));
    assert_eq!(custom.lines, vec!["Hello!", "Stay."]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn latency_summary_keeps_fast_successes_and_exposes_outliers() {
    let stats = latency(&[10000, 100, 100, 100, 100, 0, 100, 100, 100, 100]).unwrap();
    assert_eq!(stats.mean, 1080);
    assert_eq!(stats.median, 100);
    assert_eq!(stats.p90, 100);
    assert!(latency(&[]).is_none());
    let instant = latency(&[0]).unwrap();
    assert_eq!((instant.mean, instant.median, instant.p90), (0, 0, 0));
}
