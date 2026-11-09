//! Read pictures with the neural OCR and print the lines:
//!   ORT_DYLIB_PATH=…/libonnxruntime.so cargo run --release --features dynamic \
//!     --example read_pages -- <det.onnx> <rec.onnx> <picture>…
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let t = std::time::Instant::now();
    let engine = neural_ocr::Engine::load(a[1].as_ref(), a[2].as_ref(), threads, false)?;
    println!("загрузка {} мс", t.elapsed().as_millis());
    for p in &a[3..] {
        let f = manga_pages::chapter::decode(&std::fs::read(p)?).map_err(anyhow::Error::msg)?;
        let t = std::time::Instant::now();
        let _found = engine.det.find(&f)?;
        let td = t.elapsed().as_millis();
        let lines = engine.read(&f, 0.5)?;
        println!("== {} {}×{}: поиск {} мс, всё {} мс, строк {}", p.rsplit('/').next().unwrap(), f.width, f.height, td, t.elapsed().as_millis(), lines.len());
        for l in lines {
            println!("  {:.2} [{:.0},{:.0} {:.0}×{:.0}] {}", l.score, l.rect.x, l.rect.y, l.rect.w, l.rect.h, l.text);
        }
    }
    Ok(())
}
