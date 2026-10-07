//! Screen tests: folio runs in a hidden tmux, and the test reads what is
//! on the screen. Without tmux they say so and pass.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const FOLIO: &str = env!("CARGO_BIN_EXE_folio");

/// Whether `tool` is installed. `flag` is one that makes it print and leave.
fn has(tool: &str, flag: &str) -> bool { Command::new(tool).arg(flag).output().is_ok() }

/// One tmux command on the test's own server. The socket is a file in the
/// scratch folder, so it goes when the folder does.
fn tmux(sock: &str, args: &[&str]) -> String {
    let o = Command::new("tmux").args(["-S", sock, "-f", "/dev/null"]).args(args)
        .output().expect("tmux runs");
    String::from_utf8_lossy(&o.stdout).to_string()
}

/// The screen once `ready` says so, or as it stood after two seconds.
fn screen_when(sock: &str, ready: impl Fn(&str) -> bool) -> String {
    let mut s = String::new();
    for _ in 0..40 {
        s = tmux(sock, &["capture-pane", "-p"]);
        if ready(&s) { break; }
        std::thread::sleep(Duration::from_millis(50));
    }
    s
}

/// A one-page PDF that says `words`, built by hand so no tool is needed
/// to make one.
fn tiny_pdf(words: &str) -> Vec<u8> {
    let stream = format!("BT /F1 18 Tf 20 100 Td ({}) Tj ET", words);
    let objs = [
        "<</Type/Catalog/Pages 2 0 R>>".to_string(),
        "<</Type/Pages/Kids[3 0 R]/Count 1>>".to_string(),
        "<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 200]/Contents 4 0 R\
         /Resources<</Font<</F1 5 0 R>>>>>>".to_string(),
        format!("<</Length {}>>\nstream\n{}\nendstream", stream.len(), stream),
        "<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut at = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        at.push(out.len());
        out.extend(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
    for a in at { out.extend(format!("{:010} 00000 n \n", a).bytes()); }
    out.extend(format!("trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{}\n%%EOF\n",
        objs.len() + 1, xref).bytes());
    out
}

/// A scratch folder with `paper.pdf` in it and a home that opens in page
/// mode, and folio running there on the bare file name.
fn start(name: &str) -> (PathBuf, String) {
    let tmp = std::env::temp_dir().join(format!("folio-{}-{}", name, std::process::id()));
    std::fs::remove_dir_all(&tmp).ok();
    std::fs::create_dir_all(tmp.join("home/.folio")).unwrap();
    std::fs::write(tmp.join("home/.folio/config"), "mode = page\n").unwrap();
    std::fs::write(tmp.join("paper.pdf"), tiny_pdf("folio zebra")).unwrap();
    let sock = tmp.join("tmux").to_string_lossy().to_string();
    (tmp, sock)
}

fn run(tmp: &Path, sock: &str) {
    tmux(sock, &["new-session", "-d", "-x", "110", "-y", "34", "-c", &tmp.to_string_lossy(),
        "env", "-i", &format!("HOME={}", tmp.join("home").display()), "TERM=xterm-256color",
        "PATH=/usr/bin:/bin", FOLIO, "paper.pdf"]);
}

/// Help covers the whole screen, and the page view paints less of it. The
/// help text stayed around the page until something else forced a repaint.
/// The same run checks the header: the whole path, though folio was started
/// with the bare file name.
#[test]
fn help_leaves_nothing_behind() {
    if !has("tmux", "-V") { eprintln!("skipped: needs tmux"); return; }
    let (tmp, sock) = start("help");
    let full = std::fs::canonicalize(tmp.join("paper.pdf")).unwrap();
    run(&tmp, &sock);

    let page = screen_when(&sock, |s| s.contains("page 1/1"));
    tmux(&sock, &["send-keys", "?"]);
    let help = screen_when(&sock, |s| s.contains("any key to go back"));
    tmux(&sock, &["send-keys", "x"]);
    let back = screen_when(&sock, |s| s.contains("q:Quit") && !s.contains("any key to go back"));
    tmux(&sock, &["kill-server"]);
    std::fs::remove_dir_all(&tmp).ok();

    assert!(page.lines().next().unwrap_or("").contains(&*full.to_string_lossy()),
        "the header carries the whole path:\n{}", page);
    assert!(help.contains("  t p v        text / page / split"),
        "the help shows, with its indent:\n{}", help);
    for gone in ["terminal PDF reader", "cycle the modes", "Config is"] {
        assert!(!back.contains(gone), "help text left on screen ({}):\n{}", gone, back);
    }
}

/// The corpus list was written into the text pane, which is one column
/// wide in page mode: `s` found the document and showed nothing.
#[test]
fn the_corpus_list_shows_in_page_mode() {
    if !has("tmux", "-V") || !has("pdftotext", "-v") {
        eprintln!("skipped: needs tmux and pdftotext");
        return;
    }
    let (tmp, sock) = start("corpus");
    Command::new(FOLIO).arg("--index").arg(&tmp)
        .env_clear().env("HOME", tmp.join("home")).env("PATH", "/usr/bin:/bin")
        .output().expect("folio --index runs");
    run(&tmp, &sock);

    screen_when(&sock, |s| s.contains("page 1/1"));
    tmux(&sock, &["send-keys", "s"]);
    screen_when(&sock, |s| s.contains("corpus /"));
    tmux(&sock, &["send-keys", "zebra", "Enter"]);
    let list = screen_when(&sock, |s| s.contains("pick a number"));
    tmux(&sock, &["send-keys", "x"]);
    // The footer is repainted last, so the prompt going is the screen done.
    let back = screen_when(&sock, |s| !s.contains("pick a number"));
    tmux(&sock, &["kill-server"]);
    std::fs::remove_dir_all(&tmp).ok();

    assert!(list.contains("1 document(s) carry \"zebra\""), "the list has its heading:\n{}", list);
    assert!(list.contains("paper.pdf  p.1"), "the list names the document:\n{}", list);
    assert!(!back.contains("document(s) carry"), "the list left on screen:\n{}", back);
}
