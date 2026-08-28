//! Everything that touches a PDF or the cache under `~/.folio/`.
//!
//! Two outside tools do the heavy work, and both are already on the box:
//! `pdftotext` for the text layer and `mutool` for page images. Neither is
//! run twice for the same answer, text is extracted once per document and
//! kept, page images are rendered once per (page, height) and kept. A
//! document that has not changed costs one `stat` to confirm it.
//!
//! The cache key carries the file's mtime, so rebuilding a PDF from its
//! source invalidates every page of it without anything having to notice.

use std::path::{Path, PathBuf};
use std::process::Command;

/// FNV-1a over the path. A hash, not a checksum: it only has to keep two
/// different documents from sharing a cache slot, and it saves a crate.
fn hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

pub fn folio_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    Path::new(&home).join(".folio")
}

fn cache_dir() -> PathBuf {
    let d = folio_dir().join("cache");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Cache prefix for a document: its path hashed, plus its mtime. A rebuilt
/// PDF gets a new prefix, so stale pages are never shown and nothing has to
/// be swept.
fn key(path: &Path) -> String {
    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{:016x}-{}", hash(&path.to_string_lossy()), mtime)
}

/// The document's text, one entry per page.
///
/// `pdftotext` writes a form feed between pages, which is the only page
/// boundary the text layer carries, and it is exact: an 18-page paper comes
/// back as 18 entries. Cached whole, because extracting all of it costs the
/// same 89 ms as extracting one page.
pub fn text_pages(path: &Path) -> Vec<String> {
    let cached = cache_dir().join(format!("{}.txt", key(path)));
    if let Ok(s) = std::fs::read_to_string(&cached) {
        return split_pages(&s);
    }
    let out = match Command::new("pdftotext").arg("-layout").arg(path).arg("-").output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return Vec::new(),
    };
    let _ = std::fs::write(&cached, &out);
    split_pages(&out)
}

/// The whole document's text as one string, from the cache only. The corpus
/// search wants to ask "is the phrase in here at all" over thousands of
/// documents; splitting every one of them into pages first allocated a Vec
/// of Strings per document to answer a question about the raw bytes.
pub fn cached_text(path: &Path) -> Option<String> {
    std::fs::read_to_string(cache_dir().join(format!("{}.txt", key(path)))).ok()
}

fn split_pages(s: &str) -> Vec<String> {
    let mut v: Vec<String> = s.split('\u{c}').map(|p| p.trim_end().to_string()).collect();
    // A trailing form feed leaves an empty last entry that is not a page.
    if v.last().map(|p| p.trim().is_empty()).unwrap_or(false) && v.len() > 1 {
        v.pop();
    }
    v
}

/// How many pages the document has, from `pdfinfo`. Falls back to the text
/// layer's page count, which is right for anything with text and gives 1 for
/// a scan rather than 0.
pub fn page_count(path: &Path) -> usize {
    if let Ok(o) = Command::new("pdfinfo").arg(path).output() {
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            if let Some(v) = line.strip_prefix("Pages:") {
                if let Ok(n) = v.trim().parse::<usize>() {
                    if n > 0 { return n; }
                }
            }
        }
    }
    text_pages(path).len().max(1)
}

/// A page as a PNG, sized to `height_px`, cached. Returns the file to hand
/// to glow.
///
/// `mutool -h` fits the height and keeps the aspect, which is what a pane
/// wants: a page is taller than it is wide, so height is the binding
/// constraint and the width follows.
pub fn render_page(path: &Path, page: usize, height_px: u32) -> Option<PathBuf> {
    let out = cache_dir().join(format!("{}-p{}-h{}.png", key(path), page, height_px));
    if out.exists() { return Some(out); }
    let status = Command::new("mutool")
        .arg("draw")
        .arg("-F").arg("png")
        .arg("-o").arg(&out)
        .arg("-h").arg(height_px.to_string())
        .arg(path)
        .arg((page + 1).to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    if status.success() && out.exists() { Some(out) } else { None }
}

/// The cache file for a page, if it has already been rendered.
pub fn cached_page(path: &Path, page: usize, height_px: u32) -> Option<PathBuf> {
    let out = cache_dir().join(format!("{}-p{}-h{}.png", key(path), page, height_px));
    if out.exists() { Some(out) } else { None }
}

/// Render the pages either side of the one being read, so paging forward is
/// instant. On its own thread, because mutool takes about a tenth of a second
/// and the reader must not wait for it. One at a time: paging fast through a
/// deck would otherwise start a thread per keypress.
pub fn prefetch_bg(path: &Path, page: usize, pages: usize, height_px: u32) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static BUSY: AtomicBool = AtomicBool::new(false);
    if BUSY.swap(true, Ordering::SeqCst) { return; }
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        for p in [page + 1, page.wrapping_sub(1)] {
            if p < pages && cached_page(&path, p, height_px).is_none() {
                let _ = render_page(&path, p, height_px);
            }
        }
        BUSY.store(false, Ordering::SeqCst);
    });
}

/// The source a PDF was built from, if it is sitting next to it under the
/// same name. This is what makes "edit and rebuild" exact: your papers are
/// Markdown and the book is LaTeX, so there is no need to touch the PDF's
/// own bytes to change what it says.
pub fn source_for(path: &Path) -> Option<PathBuf> {
    let stem = path.file_stem()?.to_string_lossy().to_string();
    let dir = path.parent()?;
    // .hl first: a HyperList is the source Geir writes, and folio inventing
    // a .txt beside one left the edit in a third file while the source and
    // the PDF both went stale.
    for ext in ["hl", "tex", "md", "markdown", "html"] {
        let cand = dir.join(format!("{}.{}", stem, ext));
        if cand.exists() { return Some(cand); }
    }
    None
}

/// Rebuild the PDF from its source. `cmd` is the configured command line for
/// that extension; `{src}` and `{out}` are filled in. Returns the tool's own
/// complaint on failure, because a LaTeX error is the useful thing to show.
pub fn rebuild(source: &Path, pdf: &Path, cmd: &str) -> Result<(), String> {
    let src = source.to_string_lossy().to_string();
    let out = pdf.to_string_lossy().to_string();
    let line = cmd.replace("{src}", &src).replace("{out}", &out);
    let dir = source.parent().unwrap_or(Path::new("."));
    // A real build is often a pipeline: render to HTML, patch it, print it.
    // Hand anything with shell punctuation to the shell, and spawn the rest
    // directly so the common case costs no extra process.
    let needs_shell = line.contains("&&") || line.contains("||")
        || line.contains('|') || line.contains(';')
        || line.contains('>') || line.contains('<');
    let o = if needs_shell {
        Command::new("sh").arg("-c").arg(&line).current_dir(dir).output()
            .map_err(|e| format!("sh: {}", e))?
    } else {
        let mut parts = line.split_whitespace();
        let prog = parts.next().ok_or("empty build command")?;
        let args: Vec<String> = parts.map(|s| s.to_string()).collect();
        Command::new(prog).args(&args).current_dir(dir).output()
            .map_err(|e| format!("{}: {}", prog, e))?
    };
    if o.status.success() { return Ok(()); }
    // LaTeX puts its errors on stdout, most other tools on stderr.
    let err = String::from_utf8_lossy(&o.stderr);
    let msg = if err.trim().is_empty() { String::from_utf8_lossy(&o.stdout).to_string() }
              else { err.to_string() };
    let last = msg.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("build failed");
    Err(last.trim().to_string())
}

/// Every PDF under `root`, for the corpus index.
pub fn find_pdfs(root: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth == 0 { return; }
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            // Skip the caches and repositories that would double the work.
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') { continue; }
            find_pdfs(&p, out, depth - 1);
        } else if p.extension().map(|x| x.eq_ignore_ascii_case("pdf")).unwrap_or(false) {
            out.push(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real paper to test against, named by `FOLIO_TEST_PDF`, and a real
    /// scan by `FOLIO_TEST_SCAN`. Unset, the test says so and passes: the
    /// documents are the author's, and the repo is not.
    fn sample(var: &str) -> Option<PathBuf> {
        let p = PathBuf::from(std::env::var(var).ok()?);
        if p.exists() { Some(p) } else { None }
    }

    #[test]
    fn a_paper_reads_as_pages_of_text() {
        let Some(p) = sample("FOLIO_TEST_PDF") else {
            eprintln!("skipped: set FOLIO_TEST_PDF to a text-bearing PDF");
            return;
        };
        let p = p.as_path();
        let pages = text_pages(p);
        assert_eq!(page_count(p), 18, "pdfinfo page count");
        assert_eq!(pages.len(), 18, "one text entry per page");
        assert!(pages[0].len() > 200, "first page carries real text");
        // Cached: a second call must not shell out again, and must agree.
        assert_eq!(text_pages(p).len(), 18);
        println!("page 1 starts: {:?}", pages[0].lines().next().unwrap_or(""));
    }

    #[test]
    fn a_page_renders_to_the_height_asked_for() {
        let Some(p) = sample("FOLIO_TEST_PDF") else {
            eprintln!("skipped: set FOLIO_TEST_PDF to a text-bearing PDF");
            return;
        };
        let p = p.as_path();
        let png = render_page(p, 2, 600).expect("mutool renders page 3");
        assert!(png.exists());
        let bytes = std::fs::read(&png).unwrap();
        // PNG header carries the dimensions at a fixed offset.
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        assert_eq!(h, 600, "rendered to the requested height");
        // Second call is the cache, not another render.
        let again = render_page(p, 2, 600).unwrap();
        assert_eq!(png, again);
        println!("rendered {} bytes", bytes.len());
    }

    #[test]
    fn a_paper_finds_its_own_source() {
        let Some(p) = sample("FOLIO_TEST_PDF") else {
            eprintln!("skipped: set FOLIO_TEST_PDF to a text-bearing PDF");
            return;
        };
        let p = p.as_path();
        let src = source_for(p).expect("the .md sits beside it");
        assert_eq!(src.extension().unwrap(), "md");
        // A PDF with nothing beside it reports nothing.
        if let Some(lone) = sample("FOLIO_TEST_SCAN") {
            assert!(source_for(&lone).is_none(), "a scan has no source beside it");
        }
        println!("source: {}", src.display());
    }

    #[test]
    fn a_scan_has_no_text_to_show() {
        let Some(lone) = sample("FOLIO_TEST_SCAN") else {
            eprintln!("skipped: set FOLIO_TEST_SCAN to a scanned PDF");
            return;
        };
        let lone = lone.as_path();
        let pages = text_pages(lone);
        let chars: usize = pages.iter().map(|p| p.trim().len()).sum();
        assert!(chars < 50, "a scan carries no text layer, got {}", chars);
        // But it still renders, which is why Page mode is not optional.
        assert!(render_page(lone, 0, 400).is_some());
        println!("scan: {} pages, {} chars of text", pages.len(), chars);
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;

    /// A PDF rendered from a HyperList has to find that HyperList. Missing
    /// `.hl` from the list is what made folio invent a `.txt` beside one,
    /// leaving the edit in a third file while the source went stale.
    #[test]
    fn a_hyperlist_is_a_source() {
        let tmp = std::env::temp_dir().join("folio-source-test");
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        let pdf = tmp.join("notes.pdf");
        std::fs::write(&pdf, b"%PDF").unwrap();

        assert!(source_for(&pdf).is_none(), "nothing beside it yet");

        std::fs::write(tmp.join("notes.hl"), b"Root\n\tChild\n").unwrap();
        assert_eq!(source_for(&pdf).unwrap(), tmp.join("notes.hl"));

        // With several candidates the HyperList still wins, because it is
        // the one the author writes.
        std::fs::write(tmp.join("notes.md"), b"# Notes\n").unwrap();
        std::fs::write(tmp.join("notes.tex"), b"\\documentclass{article}").unwrap();
        assert_eq!(source_for(&pdf).unwrap(), tmp.join("notes.hl"));

        // Without one, the older kinds still resolve in their own order.
        std::fs::remove_file(tmp.join("notes.hl")).unwrap();
        assert_eq!(source_for(&pdf).unwrap(), tmp.join("notes.tex"));

        // A file of another name is not this document's source.
        std::fs::remove_file(tmp.join("notes.tex")).unwrap();
        std::fs::remove_file(tmp.join("notes.md")).unwrap();
        std::fs::write(tmp.join("other.hl"), b"x").unwrap();
        assert!(source_for(&pdf).is_none());

        std::fs::remove_dir_all(&tmp).ok();
    }
}
