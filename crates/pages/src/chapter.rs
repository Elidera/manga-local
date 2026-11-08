//! A chapter on disk: a folder of pictures or a ZIP / CBZ archive. Pages are
//! read one at a time (chapters of long webtoons are hundreds of megabytes) and
//! the translated chapter is written next to the original: a folder
//! «<name> (перевод)» or «<name> (перевод).cbz».

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use manga_core::image::Frame;

const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif"];

fn is_image(name: &str) -> bool {
    let lower = name.to_lowercase();
    IMAGE_EXT.iter().any(|e| lower.ends_with(&format!(".{e}")))
}

fn is_archive(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("zip") || e.eq_ignore_ascii_case("cbz"))
}

/// "2.jpg" before "10.jpg": runs of digits compare as numbers.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = a.peek().copied().filter(|c| c.is_ascii_digit()) {
                    na.push(c);
                    a.next();
                }
                let mut nb = String::new();
                while let Some(c) = b.peek().copied().filter(|c| c.is_ascii_digit()) {
                    nb.push(c);
                    b.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then(ta.cmp(tb));
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
                a.next();
                b.next();
            }
        }
    }
}

pub struct Chapter {
    /// Shown to the reader and used for the output name.
    pub name: String,
    source: Source,
    /// Page names in reading order (file names, or paths inside the archive).
    pub pages: Vec<String>,
}

enum Source {
    Folder(PathBuf),
    Archive(PathBuf),
}

impl Chapter {
    /// `path`: a folder, a ZIP / CBZ archive, or one picture (then its whole folder).
    pub fn open(path: &Path) -> Result<Chapter, String> {
        let shown = |p: &Path| p.file_stem().or(p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "глава".into());
        if is_archive(path) {
            let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let z = zip::ZipArchive::new(f).map_err(|e| format!("архив {}: {e}", path.display()))?;
            let mut pages: Vec<String> = z.file_names().filter(|n| is_image(n) && !n.starts_with("__MACOSX")).map(String::from).collect();
            pages.sort_by(|a, b| natural_cmp(a, b));
            return Ok(Chapter { name: shown(path), source: Source::Archive(path.to_path_buf()), pages });
        }
        let dir = if path.is_dir() { path.to_path_buf() } else { path.parent().map(Path::to_path_buf).ok_or("нет папки")? };
        let mut pages: Vec<String> = std::fs::read_dir(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .flatten()
            .filter(|e| e.path().is_file())
            .filter_map(|e| e.file_name().to_str().map(String::from))
            .filter(|n| is_image(n))
            .collect();
        pages.sort_by(|a, b| natural_cmp(a, b));
        Ok(Chapter { name: shown(&dir), source: Source::Folder(dir), pages })
    }

    pub fn read(&self, i: usize) -> Result<Vec<u8>, String> {
        let name = &self.pages[i];
        match &self.source {
            Source::Folder(d) => std::fs::read(d.join(name)).map_err(|e| format!("{name}: {e}")),
            Source::Archive(p) => {
                let f = std::fs::File::open(p).map_err(|e| e.to_string())?;
                let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
                let mut entry = z.by_name(name).map_err(|e| format!("{name}: {e}"))?;
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf).map_err(|e| format!("{name}: {e}"))?;
                Ok(buf)
            }
        }
    }

    /// Where the translation goes: next to the original, never over it.
    pub fn output_path(&self) -> PathBuf {
        match &self.source {
            Source::Folder(d) => sibling(d, &format!("{} (перевод)", self.name), ""),
            Source::Archive(p) => sibling(p, &format!("{} (перевод)", self.name), "cbz"),
        }
    }

    pub fn is_archive(&self) -> bool {
        matches!(self.source, Source::Archive(_))
    }
}

/// `<parent>/<stem>[.ext]`, with « 2», « 3»… when it already exists.
fn sibling(p: &Path, stem: &str, ext: &str) -> PathBuf {
    let parent = p.parent().unwrap_or(Path::new("."));
    let make = |n: usize| {
        let s = if n == 1 { stem.to_string() } else { format!("{stem} {n}") };
        parent.join(if ext.is_empty() { s } else { format!("{s}.{ext}") })
    };
    (1..).map(make).find(|c| !c.exists()).unwrap()
}

/// Decode a picture into a BGRA frame.
pub fn decode(bytes: &[u8]) -> Result<Frame, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("картинка: {e}"))?.to_rgba8();
    let (w, h) = img.dimensions();
    let mut bgra = img.into_raw();
    for p in bgra.chunks_exact_mut(4) {
        p.swap(0, 2);
        p[3] = 255;
    }
    Ok(Frame::new(w, h, bgra))
}

/// Encode a frame: JPEG (quality 92) for pages that were JPEG, PNG otherwise.
pub fn encode(f: &Frame, as_jpeg: bool) -> Result<Vec<u8>, String> {
    let mut rgb = Vec::with_capacity((f.width * f.height * 3) as usize);
    for p in f.bgra.chunks_exact(4) {
        rgb.extend_from_slice(&[p[2], p[1], p[0]]);
    }
    let mut out = Vec::new();
    let res = if as_jpeg {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92).encode(&rgb, f.width, f.height, image::ExtendedColorType::Rgb8)
    } else {
        use image::ImageEncoder;
        image::codecs::png::PngEncoder::new(&mut out).write_image(&rgb, f.width, f.height, image::ExtendedColorType::Rgb8)
    };
    res.map_err(|e| format!("сохранение: {e}"))?;
    Ok(out)
}

/// The output name of a page: JPEG pages stay .jpg, the rest become .png.
pub fn output_name(page: &str) -> (String, bool) {
    let base = page.rsplit(['/', '\\']).next().unwrap_or(page);
    let lower = base.to_lowercase();
    let jpeg = lower.ends_with(".jpg") || lower.ends_with(".jpeg");
    let stem = base.rsplit_once('.').map_or(base, |(s, _)| s);
    (if jpeg { format!("{stem}.jpg") } else { format!("{stem}.png") }, jpeg)
}

/// Writes the translated pages: into a folder, or into a CBZ archive.
pub enum Output {
    Folder(PathBuf),
    Cbz(zip::ZipWriter<std::fs::File>),
}

impl Output {
    pub fn create(path: &Path, cbz: bool) -> Result<Output, String> {
        if cbz {
            let f = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(Output::Cbz(zip::ZipWriter::new(f)))
        } else {
            std::fs::create_dir_all(path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(Output::Folder(path.to_path_buf()))
        }
    }

    pub fn add(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        match self {
            Output::Folder(d) => std::fs::write(d.join(name), bytes).map_err(|e| format!("{name}: {e}")),
            Output::Cbz(z) => {
                // Pictures are already compressed: stored as they are.
                let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
                z.start_file(name, opts).map_err(|e| e.to_string())?;
                z.write_all(bytes).map_err(|e| e.to_string())
            }
        }
    }

    pub fn finish(self) -> Result<(), String> {
        match self {
            Output::Folder(_) => Ok(()),
            Output::Cbz(z) => z.finish().map(|_| ()).map_err(|e| e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["10.jpg", "2.jpg", "1.jpg", "ch1/003.png", "ch1/01.png"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["1.jpg", "2.jpg", "10.jpg", "ch1/01.png", "ch1/003.png"]);
    }

    #[test]
    fn output_names() {
        assert_eq!(output_name("ch/001.JPG"), ("001.jpg".into(), true));
        assert_eq!(output_name("002.webp"), ("002.png".into(), false));
    }

    #[test]
    fn folder_and_cbz_round_trip() {
        let dir = std::env::temp_dir().join(format!("manga-pages-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("Глава 1");
        std::fs::create_dir_all(&src).unwrap();
        let f = Frame::new(4, 3, vec![200; 48]);
        for n in ["10.png", "2.png", "notes.txt"] {
            std::fs::write(src.join(n), encode(&f, false).unwrap()).unwrap();
        }
        let ch = Chapter::open(&src.join("2.png")).unwrap();
        assert_eq!(ch.pages, ["2.png", "10.png"]);
        assert_eq!(ch.output_path(), dir.join("Глава 1 (перевод)"));
        let back = decode(&ch.read(1).unwrap()).unwrap();
        assert_eq!((back.width, back.height), (4, 3));

        let cbz = dir.join("Глава 1 (перевод).cbz");
        let mut out = Output::create(&cbz, true).unwrap();
        out.add("2.png", &ch.read(0).unwrap()).unwrap();
        out.finish().unwrap();
        let again = Chapter::open(&cbz).unwrap();
        assert_eq!(again.pages, ["2.png"]);
        assert!(again.output_path().ends_with("Глава 1 (перевод) (перевод).cbz"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
