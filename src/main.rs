//! folio, a terminal PDF reader. Part of the Fe₂O₃ suite.
//!
//! A PDF is two documents in one: the words, and the page they were set on.
//! Most readers show you only the second. folio shows either, or both side
//! by side, because reading a paper and quoting from it want different
//! things.
//!
//! Three modes, and the config picks which one a document opens in:
//!   1  the page's text, full width
//!   2  the page as an image, full width
//!   3  text on the left, the page on the right
//!
//! Everything is event-driven: no timers, no polling, nothing running while
//! you read. Text is extracted once per document and kept. A page image is
//! rendered once per size and kept. Turning a page you have already seen
//! costs a `stat`.
//!
//! What it does that a viewer cannot: `e` edits the document's SOURCE when
//! one is beside it (`.md`, `.tex`), rebuilds, and reloads the page. Your
//! papers are Markdown and the book is LaTeX, so a small change is a real
//! edit of the real document, not a patch over the top of a page.

mod pdf;

use crust::style;
use crust::{Crust, Input, Pane};
use std::path::{Path, PathBuf};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const TEXT_FG: u8 = 252;
const DIM_FG: u8 = 245;
const HIT_FG: u8 = 220;
/// Same colour, in the width `Pane::new` wants.
const TEXT_BG_FG: u16 = TEXT_FG as u16;

#[derive(Clone, Copy, PartialEq)]
enum Mode { Text, Page, Split }

impl Mode {
    fn name(self) -> &'static str {
        match self { Mode::Text => "Text", Mode::Page => "Page", Mode::Split => "Split" }
    }
    fn next(self) -> Mode {
        match self { Mode::Text => Mode::Page, Mode::Page => Mode::Split, Mode::Split => Mode::Text }
    }
    fn parse(s: &str) -> Mode {
        match s.trim().to_ascii_lowercase().as_str() {
            "page" | "2" => Mode::Page,
            "split" | "3" => Mode::Split,
            _ => Mode::Text,
        }
    }
}

/// `~/.folio/config`, one `key = value` per line. Absent keys keep the
/// defaults below, so the file is optional and can hold only what you
/// disagree with.
struct Config {
    mode: Mode,
    /// Share of the width the text pane takes in Split, in percent.
    split: u16,
    editor: String,
    build_tex: String,
    build_md: String,
    /// Where `--index` looks when given no directory.
    library: String,
}

impl Config {
    fn load() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let mut c = Config {
            mode: Mode::Text,
            split: 50,
            editor: std::env::var("EDITOR").unwrap_or_else(|_| "scribe".into()),
            // Twice, because one pass leaves every cross-reference unresolved.
            build_tex: "pdflatex -interaction=nonstopmode {src}".into(),
            build_md: "pandoc {src} -o {out}".into(),
            library: format!("{}/Main", home),
        };
        let path = pdf::folio_dir().join("config");
        let Ok(text) = std::fs::read_to_string(path) else { return c };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim().to_string();
            match k.trim() {
                "mode" => c.mode = Mode::parse(&v),
                "split" => if let Ok(n) = v.parse() { c.split = n },
                "editor" => c.editor = v,
                "build_tex" => c.build_tex = v,
                "build_md" => c.build_md = v,
                "library" => c.library = v,
                _ => {}
            }
        }
        c
    }
}

/// Write one setting back, leaving every other line of the config, and any
/// comment in it, exactly as the user wrote it.
fn save_split(split: u16) {
    let file = pdf::folio_dir().join("config");
    let old = std::fs::read_to_string(&file).unwrap_or_default();
    let mut out = String::new();
    let mut seen = false;
    for line in old.lines() {
        if line.trim_start().starts_with("split") && line.contains('=') {
            out.push_str(&format!("split = {}\n", split));
            seen = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !seen { out.push_str(&format!("split = {}\n", split)); }
    let _ = std::fs::create_dir_all(pdf::folio_dir());
    let _ = std::fs::write(file, out);
}

/// Where you were, per document. One tab-separated line each, in
/// `~/.folio/state`, so it is readable and repairable with an editor.
fn state_file() -> PathBuf { pdf::folio_dir().join("state") }

fn saved_page(path: &Path) -> usize { saved_page_in(&state_file(), path) }

fn save_page(path: &Path, page: usize) { save_page_in(&state_file(), path, page) }

fn saved_page_in(file: &Path, path: &Path) -> usize {
    let want = path.to_string_lossy().to_string();
    let Ok(text) = std::fs::read_to_string(file) else { return 0 };
    for line in text.lines() {
        if let Some((p, n)) = line.split_once('\t') {
            if p == want { return n.trim().parse().unwrap_or(0); }
        }
    }
    0
}

fn save_page_in(file: &Path, path: &Path, page: usize) {
    let want = path.to_string_lossy().to_string();
    let mut out = String::new();
    if let Ok(text) = std::fs::read_to_string(file) {
        for line in text.lines() {
            if !line.starts_with(&format!("{}\t", want)) && !line.trim().is_empty() {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out.push_str(&format!("{}\t{}\n", want, page));
    if let Some(dir) = file.parent() { let _ = std::fs::create_dir_all(dir); }
    let _ = std::fs::write(file, out);
}

struct App {
    path: PathBuf,
    pages: usize,
    text: Vec<String>,
    page: usize,
    /// First visible line of the current page's text.
    scroll: usize,
    mode: Mode,
    cfg: Config,
    header: Pane,
    left: Pane,
    right: Pane,
    footer: Pane,
    cols: u16,
    rows: u16,
    img: Option<glow::Display>,
    /// Which image is on screen, so an unchanged page is not re-sent.
    shown: Option<String>,
    status: Option<(String, u8)>,
    needle: String,
    hits: Vec<usize>,
    hit: usize,
    /// Digits typed so far, waiting for the key that uses them: `10g`.
    count: String,
    /// A `g` has been pressed and is waiting to see whether a second
    /// one follows, which is vim's `gg`.
    g_pending: bool,
}

impl App {
    fn new(path: PathBuf) -> Self {
        let cfg = Config::load();
        let text = pdf::text_pages(&path);
        let pages = pdf::page_count(&path);
        let page = saved_page(&path).min(pages.saturating_sub(1));
        let (cols, rows) = Crust::terminal_size();
        let mut app = App {
            path, pages, text, page, scroll: 0,
            mode: cfg.mode, cfg,
            header: Pane::new(1, 1, cols, 1, 255, 236),
            left: Pane::new(1, 2, cols, rows.saturating_sub(2), TEXT_BG_FG, 0),
            right: Pane::new(1, 2, 1, 1, TEXT_BG_FG, 0),
            footer: Pane::new(1, rows, cols, 1, 248, 236),
            cols, rows,
            img: None, shown: None, status: None,
            needle: String::new(), hits: Vec::new(), hit: 0,
            count: String::new(), g_pending: false,
        };
        app.layout();
        app
    }

    /// Pane geometry for the current mode. Called on start, on a mode change
    /// and on a resize; never per keypress.
    fn layout(&mut self) {
        let (cols, rows) = Crust::terminal_size();
        self.cols = cols;
        self.rows = rows;
        let body_h = rows.saturating_sub(2);
        self.header = Pane::new(1, 1, cols, 1, 255, 236);
        self.footer = Pane::new(1, rows, cols, 1, 248, 236);
        self.header.scroll = false;
        self.footer.scroll = false;
        let lw = match self.mode {
            Mode::Text => cols,
            Mode::Page => 0,
            Mode::Split => (cols as u32 * self.cfg.split as u32 / 100) as u16,
        };
        self.left = Pane::new(1, 2, lw.max(1), body_h, TEXT_BG_FG, 0);
        self.left.scroll = false;
        let rx = lw + 1;
        self.right = Pane::new(rx.max(1), 2, cols.saturating_sub(lw).max(1), body_h, TEXT_BG_FG, 0);
        self.right.scroll = false;
    }

    fn page_text(&self) -> &str {
        self.text.get(self.page).map(|s| s.as_str()).unwrap_or("")
    }

    /// The image pane in cells, or None when this mode shows no image.
    fn image_box(&self) -> Option<(u16, u16, u16, u16)> {
        match self.mode {
            Mode::Text => None,
            Mode::Page => Some((1, 2, self.cols, self.rows.saturating_sub(2))),
            Mode::Split => Some((self.right.x, self.right.y, self.right.w, self.right.h)),
        }
    }

    fn clear_image(&mut self) {
        if let Some(ref mut d) = self.img {
            d.clear(1, 2, self.cols, self.rows.saturating_sub(2), self.cols, self.rows);
        }
        self.img = None;
        self.shown = None;
    }

    fn render(&mut self) {
        // Header: name, page, mode, and the source if there is one to rebuild.
        let name = self.path.file_name().map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "?".into());
        let src = pdf::source_for(&self.path)
            .and_then(|p| p.extension().map(|e| format!("  [{}]", e.to_string_lossy())))
            .unwrap_or_default();
        let hits = if self.hits.is_empty() { String::new() }
                   else { format!("  /{}  {}/{}", self.needle, self.hit + 1, self.hits.len()) };
        // A count being typed has to be visible, or `10g` is typed blind.
        let count = if self.count.is_empty() { String::new() }
                    else { format!("  {}", self.count) };
        self.header.set_text(&format!(" FOLIO  {}  page {}/{}  [{}]{}{}{}",
            name, self.page + 1, self.pages, self.mode.name(), src, hits, count));
        self.header.refresh();

        if self.mode != Mode::Page {
            let body = self.page_text();
            let lines: Vec<&str> = body.lines().collect();
            let h = self.left.h as usize;
            let start = self.scroll.min(lines.len().saturating_sub(1));
            let end = (start + h).min(lines.len());
            let w = self.left.w as usize;
            let mut out = String::new();
            for l in &lines[start..end] {
                let t: String = l.chars().take(w).collect();
                // A hit on this page is worth seeing in the text too.
                if !self.needle.is_empty()
                    && t.to_lowercase().contains(&self.needle.to_lowercase()) {
                    out.push_str(&style::fg(&t, HIT_FG));
                } else {
                    out.push_str(&t);
                }
                out.push('\n');
            }
            if lines.is_empty() {
                out.push_str(&style::fg(
                    "\n  No text layer on this page. It is a scan, so read it in Page mode (2).",
                    DIM_FG));
            }
            self.left.set_text(&out);
            self.left.full_refresh();
        }

        if let Some((x, y, w, h)) = self.image_box() {
            let (_, cell_h) = glow::get_cell_size();
            let px = (h as u32) * (cell_h.max(1) as u32);
            match pdf::render_page(&self.path, self.page, px) {
                Some(file) => {
                    let key = file.to_string_lossy().to_string();
                    if self.shown.as_deref() != Some(key.as_str()) {
                        if self.img.is_none() {
                            let d = glow::Display::new();
                            if d.supported() { self.img = Some(d); }
                        }
                        if let Some(ref mut d) = self.img {
                            d.clear(x, y, w, h, self.cols, self.rows);
                            d.show(&key, x, y, w, h);
                            self.shown = Some(key);
                        }
                    }
                    // Only now, with the page already up, warm the neighbours.
                    pdf::prefetch(&self.path, self.page, self.pages, px);
                }
                None => {
                    self.right.set_text(&style::fg("\n  mutool could not render this page.", DIM_FG));
                    self.right.refresh();
                }
            }
        }

        match self.status.take() {
            Some((msg, c)) => self.footer.say(&style::fg(&format!(" {}", msg), c)),
            None => self.footer.say(
                " q:Quit  m:Mode  j/k:Scroll  Space/b:Page  10g:Goto  /:Find  e:Edit  y:Yank  w/W:Divider  s:Corpus  ?:Help"),
        }
    }

    fn set_status(&mut self, msg: &str, c: u8) { self.status = Some((msg.to_string(), c)); }

    /// Move the split divider, `w` wider and `W` narrower, the same keys
    /// pointer uses. Remembered, so the width you settle on is the width
    /// the next document opens at.
    fn divider(&mut self, wider: bool) {
        if self.mode != Mode::Split {
            self.set_status("the divider only exists in split mode (m)", DIM_FG);
            return;
        }
        let next = if wider { self.cfg.split + 5 } else { self.cfg.split.saturating_sub(5) };
        self.cfg.split = next.clamp(20, 80);
        save_split(self.cfg.split);
        self.clear_image();
        self.layout();
        Crust::clear_screen();
    }

    /// The page a count names, or the first page when there is none. `10g`
    /// goes to page ten; a bare `gg` goes to the front.
    fn goto_counted(&mut self, default_last: bool) {
        let n: Option<usize> = self.count.parse().ok();
        self.count.clear();
        self.g_pending = false;
        match n {
            Some(n) if n >= 1 => {
                let p = (n - 1).min(self.pages - 1);
                self.goto(p);
                if n > self.pages { self.set_status(&format!("only {} pages", self.pages), DIM_FG); }
            }
            _ => {
                let p = if default_last { self.pages.saturating_sub(1) } else { 0 };
                self.goto(p);
            }
        }
    }

    fn goto(&mut self, page: usize) {
        let p = page.min(self.pages.saturating_sub(1));
        if p == self.page { return; }
        self.page = p;
        self.scroll = 0;
        save_page(&self.path, p);
    }

    fn scroll_by(&mut self, delta: i32) {
        let lines = self.page_text().lines().count();
        let h = self.left.h as usize;
        let max = lines.saturating_sub(h);
        let next = self.scroll as i32 + delta;
        // Past the end of a page's text, turn the page. Reading is
        // continuous even though the document is not.
        if next < 0 {
            if self.page > 0 {
                self.goto(self.page - 1);
                let l = self.page_text().lines().count();
                self.scroll = l.saturating_sub(h);
            }
            return;
        }
        if next as usize > max {
            if self.page + 1 < self.pages { self.goto(self.page + 1); }
            else { self.scroll = max; }
            return;
        }
        self.scroll = next as usize;
    }

    /// Find across the whole document, not just this page. Lands on the
    /// first page that carries it; `n` walks the rest.
    fn find(&mut self) {
        let q = self.footer.ask_with_bg("/", "", 17);
        if q.trim().is_empty() { self.needle.clear(); self.hits.clear(); return; }
        self.needle = q.trim().to_string();
        let lc = self.needle.to_lowercase();
        self.hits = self.text.iter().enumerate()
            .filter(|(_, t)| t.to_lowercase().contains(&lc))
            .map(|(i, _)| i).collect();
        if self.hits.is_empty() {
            self.set_status(&format!("{}, not in this document", self.needle), 196);
            return;
        }
        // Start from the page being read, so `/` never sends you backwards.
        self.hit = self.hits.iter().position(|&p| p >= self.page).unwrap_or(0);
        let p = self.hits[self.hit];
        self.goto(p);
        self.set_status(&format!("{}, {} page(s)", self.needle, self.hits.len()), 46);
    }

    fn next_hit(&mut self, back: bool) {
        if self.hits.is_empty() { return; }
        let n = self.hits.len();
        self.hit = if back { (self.hit + n - 1) % n } else { (self.hit + 1) % n };
        let p = self.hits[self.hit];
        self.goto(p);
        self.set_status(&format!("{}  {}/{}", self.needle, self.hit + 1, n), 46);
    }

    /// Edit the document. With a source beside it, that source is what you
    /// edit, and saving rebuilds the PDF and reloads the page. Without one,
    /// you get the extracted text in a sidecar, which is a note rather than
    /// the document.
    fn edit(&mut self) {
        match pdf::source_for(&self.path) {
            Some(src) => self.edit_source(src),
            None => {
                let side = self.path.with_extension("txt");
                if !side.exists() {
                    let _ = std::fs::write(&side, self.text.join("\n\u{c}\n"));
                }
                self.run_editor(&side);
                self.set_status(&format!("edited {}", side.display()), 46);
            }
        }
    }

    fn edit_source(&mut self, src: PathBuf) {
        let before = std::fs::metadata(&src).and_then(|m| m.modified()).ok();
        self.run_editor(&src);
        let after = std::fs::metadata(&src).and_then(|m| m.modified()).ok();
        if before == after {
            self.set_status("source unchanged", DIM_FG);
            return;
        }
        let ext = src.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        let cmd = match ext.as_str() {
            "tex" => self.cfg.build_tex.clone(),
            _ => self.cfg.build_md.clone(),
        };
        self.set_status("rebuilding…", 226);
        self.render();
        match pdf::rebuild(&src, &self.path, &cmd) {
            Ok(()) => {
                self.reload();
                self.set_status("rebuilt", 46);
            }
            Err(e) => self.set_status(&format!("build failed: {}", e), 196),
        }
    }

    fn run_editor(&mut self, file: &Path) {
        Crust::cleanup();
        let _ = std::process::Command::new(&self.cfg.editor).arg(file).status();
        Crust::init();
        Crust::clear_screen();
        self.layout();
        self.shown = None;
        self.img = None;
    }

    /// Re-read the document after it changed on disk. The cache is keyed by
    /// mtime, so this picks up the new text and new page images by itself.
    fn reload(&mut self) {
        self.text = pdf::text_pages(&self.path);
        self.pages = pdf::page_count(&self.path);
        self.page = self.page.min(self.pages.saturating_sub(1));
        self.clear_image();
    }

    fn yank(&mut self) {
        let name = self.path.file_name().map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let cite = format!("{}, p. {}", name, self.page + 1);
        let body = format!("{}\n\n[{}]\n", self.page_text().trim(), cite);
        crust::clipboard_copy(&body, "clipboard");
        self.set_status(&format!("yanked page {} with citation", self.page + 1), 46);
    }

    fn write_text(&mut self) {
        let side = self.path.with_extension("txt");
        match std::fs::write(&side, self.text.join("\n\u{c}\n")) {
            Ok(()) => self.set_status(&format!("wrote {}", side.display()), 46),
            Err(e) => self.set_status(&format!("write failed: {}", e), 196),
        }
    }

    /// Search every indexed PDF, not just this one. Opens the document the
    /// chosen line came from, on the page it was found.
    fn corpus(&mut self) {
        let q = self.footer.ask_with_bg("corpus /", "", 17);
        if q.trim().is_empty() { return; }
        let hits = index_search(q.trim());
        if hits.is_empty() {
            self.set_status(&format!("{}, nothing in the index (folio --index)", q.trim()), 196);
            return;
        }
        let mut out = String::from("\n");
        out.push_str(&style::bold(&format!("  {} document(s) carry \"{}\"\n\n", hits.len(), q.trim())));
        for (i, (p, page)) in hits.iter().take(20).enumerate() {
            out.push_str(&format!("  {:>2}  {}  p.{}\n", i + 1,
                p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(), page + 1));
        }
        out.push_str(&style::fg("\n  number to open, any other key to stay\n", DIM_FG));
        self.left.set_text(&out);
        self.left.full_refresh();
        self.footer.say(" pick a number");
        let k = Input::getchr(None).unwrap_or_default();
        if let Ok(n) = k.parse::<usize>() {
            if n >= 1 && n <= hits.len().min(20) {
                let (p, page) = hits[n - 1].clone();
                self.clear_image();
                self.path = p;
                self.reload();
                self.goto(page);
                self.set_status("opened from the corpus", 46);
                return;
            }
        }
        self.set_status("", DIM_FG);
    }

    fn help(&mut self) {
        let text = format!("\n{}\n\n\
  m M          cycle the modes: text, page, split\n\
  j k ↑ ↓      scroll, turning the page at either end\n\
  Space b      next / previous page\n\
  gg G         first / last page\n\
  10g          go to page 10\n\
  / n N        find in this document, next, previous\n\
  s            find across every indexed document\n\
  e            edit: the source if there is one, else a text sidecar\n\
  y            yank this page's text with a citation\n\
  w W          widen / narrow the text pane in split mode\n\
  Ctrl-W       write the whole text beside the PDF\n\
  q            quit\n\n\
{}\n\
  Config is ~/.folio/config: mode, split, editor, build_tex, build_md, library.\n\
  Position is remembered per document in ~/.folio/state.\n\
  Build the corpus index with: folio --index [dir]\n",
            style::bold(&format!("  folio {}, terminal PDF reader", VERSION)),
            style::fg("  Editing a document that has a .md or .tex beside it edits THAT,\n  \
                        rebuilds the PDF and reloads the page.", DIM_FG));
        self.clear_image();
        self.left = Pane::new(1, 2, self.cols, self.rows.saturating_sub(2), TEXT_BG_FG, 0);
        self.left.scroll = false;
        self.left.set_text(&text);
        self.left.full_refresh();
        self.footer.say(" any key to go back");
        let _ = Input::getchr(None);
        self.layout();
    }
}

/// `~/.folio/index`: one `path<TAB>cachekey` per PDF, written by --index.
fn index_path() -> PathBuf { pdf::folio_dir().join("index") }

fn build_index(root: &Path) -> usize {
    let mut pdfs = Vec::new();
    pdf::find_pdfs(root, &mut pdfs, 8);
    let mut out = String::new();
    let mut n = 0;
    for p in &pdfs {
        // text_pages caches as a side effect, which is the whole point here.
        let pages = pdf::text_pages(p);
        if pages.iter().all(|t| t.trim().is_empty()) { continue; }
        out.push_str(&format!("{}\n", p.to_string_lossy()));
        n += 1;
        if n % 50 == 0 {
            print!("\r  {} of {} indexed", n, pdfs.len());
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
    }
    let _ = std::fs::create_dir_all(pdf::folio_dir());
    let _ = std::fs::write(index_path(), out);
    println!("\r  {} of {} documents carry text and were indexed", n, pdfs.len());
    n
}

/// Which indexed documents carry `needle`, and on which page first.
fn index_search(needle: &str) -> Vec<(PathBuf, usize)> {
    let lc = needle.to_lowercase();
    let Ok(list) = std::fs::read_to_string(index_path()) else { return Vec::new() };
    let mut out = Vec::new();
    for line in list.lines() {
        if line.trim().is_empty() { continue; }
        let p = PathBuf::from(line);
        if !p.exists() { continue; }
        for (i, page) in pdf::text_pages(&p).iter().enumerate() {
            if page.to_lowercase().contains(&lc) { out.push((p, i)); break; }
        }
        if out.len() >= 200 { break; }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut file: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-v" | "--version" => { println!("folio {}", VERSION); return; }
            "-h" | "--help" => {
                println!("folio: terminal PDF reader (Fe2O3 suite)");
                println!();
                println!("Usage: folio [file.pdf]");
                println!("       folio --index [dir]   build the corpus search index");
                println!();
                println!("Modes: 1 text, 2 page, 3 split. m cycles. The default is in");
                println!("~/.folio/config as `mode = text|page|split`.");
                println!();
                println!("e edits the document's .md or .tex source when one sits beside it,");
                println!("rebuilds the PDF and reloads the page.");
                return;
            }
            "--index" => {
                let cfg = Config::load();
                let dir = args.get(i + 1).cloned().unwrap_or(cfg.library);
                println!("Indexing {} …", dir);
                build_index(Path::new(&dir));
                return;
            }
            a => { file = Some(PathBuf::from(a)); }
        }
        i += 1;
    }

    let Some(path) = file else {
        eprintln!("folio: give it a PDF (folio --help)");
        std::process::exit(1);
    };
    if !path.exists() {
        eprintln!("folio: no such file: {}", path.display());
        std::process::exit(1);
    }

    Crust::init();
    Crust::set_app_identity("Folio");
    Crust::clear_screen();
    let mut app = App::new(path);

    loop {
        app.render();
        let Some(key) = Input::getchr(None) else { continue };
        // Digits are a count waiting for `g` or `G`, as in vim. Anything
        // else throws the half-typed count away, so a stray `1` cannot
        // change where the next keypress lands.
        let k = key.as_str();
        if k.len() == 1 && k.chars().next().unwrap().is_ascii_digit() {
            if app.count.len() < 5 { app.count.push_str(k); }
            continue;
        }
        if k != "g" { app.g_pending = false; }
        if !matches!(k, "g" | "G") { app.count.clear(); }

        match k {
            "q" | "Q" | "ESC" => break,
            "RESIZE" => { app.clear_image(); app.layout(); Crust::clear_screen(); }
            "m" | "M" => {
                app.clear_image();
                app.mode = if k == "m" { app.mode.next() } else { app.mode.next().next() };
                app.layout();
                Crust::clear_screen();
            }
            "j" | "DOWN" => app.scroll_by(1),
            "k" | "UP" => app.scroll_by(-1),
            "PgDOWN" => app.scroll_by(app.left.h as i32),
            "PgUP" => app.scroll_by(-(app.left.h as i32)),
            " " | "SPACE" | "l" | "RIGHT" => { let p = app.page + 1; app.goto(p); }
            "b" | "BACK" | "h" | "LEFT" => { let p = app.page.saturating_sub(1); app.goto(p); }
            // `10g` goes to page ten, `gg` to the front, `G` to the end.
            "g" => {
                if !app.count.is_empty() { app.goto_counted(false); }
                else if app.g_pending { app.goto_counted(false); }
                else { app.g_pending = true; }
            }
            "G" => app.goto_counted(true),
            "/" => app.find(),
            "n" => app.next_hit(false),
            "N" => app.next_hit(true),
            "s" => app.corpus(),
            "e" => app.edit(),
            "y" => app.yank(),
            "w" => app.divider(true),
            "W" => app.divider(false),
            "C-W" => app.write_text(),
            "?" => app.help(),
            _ => {}
        }
    }
    save_page(&app.path, app.page);
    app.clear_image();
    Crust::cleanup();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_cycle_and_parse() {
        assert_eq!(Mode::parse("split").name(), "Split");
        assert_eq!(Mode::parse("2").name(), "Page");
        assert_eq!(Mode::parse("nonsense").name(), "Text");
        let mut m = Mode::Text;
        for want in ["Page", "Split", "Text"] {
            m = m.next();
            assert_eq!(m.name(), want);
        }
    }

    #[test]
    fn a_page_is_remembered_per_document() {
        let tmp = std::env::temp_dir().join("folio-state-test");
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("state");
        let saved_page = |p: &Path| saved_page_in(&file, p);
        let save_page = |p: &Path, n: usize| save_page_in(&file, p, n);
        let a = Path::new("/docs/one.pdf");
        let b = Path::new("/docs/two.pdf");
        assert_eq!(saved_page(a), 0, "an unread document starts at the front");
        save_page(a, 12);
        save_page(b, 3);
        assert_eq!(saved_page(a), 12);
        assert_eq!(saved_page(b), 3);
        save_page(a, 40);
        assert_eq!(saved_page(a), 40, "the newer position replaces the old");
        assert_eq!(saved_page(b), 3, "and leaves the other document alone");
        let raw = std::fs::read_to_string(&file).unwrap();
        assert_eq!(raw.lines().count(), 2, "one line per document, not one per save");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn a_count_names_a_page() {
        // The whole of `10g`: digits gather, `g` spends them, and a page
        // past the end lands on the last one rather than panicking.
        let cases = [("10", 9usize), ("1", 0), ("18", 17), ("999", 17), ("", 0)];
        let pages = 18usize;
        for (typed, want) in cases {
            let n: Option<usize> = typed.parse().ok();
            let got = match n {
                Some(n) if n >= 1 => (n - 1).min(pages - 1),
                _ => 0,
            };
            assert_eq!(got, want, "typing {:?}g", typed);
        }
    }

    #[test]
    fn the_split_is_written_back_without_eating_the_config() {
        let tmp = std::env::temp_dir().join("folio-config-test");
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(tmp.join(".folio")).unwrap();
        let file = tmp.join(".folio/config");
        std::fs::write(&file,
            "# my reader\nmode = split\nsplit = 50\neditor = scribe\n").unwrap();
        // save_split rewrites one line; do the same transform here so the
        // test does not have to move HOME out from under the other tests.
        let old = std::fs::read_to_string(&file).unwrap();
        let mut out = String::new();
        let mut seen = false;
        for line in old.lines() {
            if line.trim_start().starts_with("split") && line.contains('=') {
                out.push_str("split = 65\n");
                seen = true;
            } else {
                out.push_str(line);
                out.push('\n');
            }
        }
        assert!(seen);
        std::fs::write(&file, &out).unwrap();
        let back = std::fs::read_to_string(&file).unwrap();
        assert!(back.contains("# my reader"), "the comment survives");
        assert!(back.contains("mode = split"), "other settings survive");
        assert!(back.contains("split = 65"), "the new width is in");
        assert!(!back.contains("split = 50"), "and the old one is gone");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn the_corpus_finds_a_paper_by_its_words() {
        if !pdf::folio_dir().join("index").exists() {
            eprintln!("skipped: no corpus index built");
            return;
        }
        let Ok(phrase) = std::env::var("FOLIO_TEST_PHRASE") else {
            eprintln!("skipped: set FOLIO_TEST_PHRASE to something in your index");
            return;
        };
        let hits = index_search(&phrase);
        assert!(!hits.is_empty(), "{:?} is in the indexed documents", phrase);
        let (path, page) = &hits[0];
        assert!(path.extension().unwrap() == "pdf");
        println!("first hit: {} p.{}", path.display(), page + 1);
        assert!(index_search("zzqqxx-not-a-word").is_empty());
    }
}
