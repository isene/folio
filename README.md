# folio

<img src="img/folio.svg" align="right" width="150">

**The terminal PDF reader. Written in Rust.**

![Rust](https://img.shields.io/badge/language-Rust-f74c00) ![License](https://img.shields.io/badge/license-Unlicense-green) ![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS-blue) ![Stay Amazing](https://img.shields.io/badge/Stay-Amazing-important)

A PDF is two documents in one: the words, and the page they were set on. Most readers show only the second. folio shows either, or both at once, because reading a paper and quoting from it want different things. Built on [Crust](https://github.com/isene/crust) and [Glow](https://github.com/isene/glow), part of the [Fe2O3 suite](https://github.com/isene/fe2o3).

## Features

- **Read a page at full width**: `z` in page mode blows the page up until it is as wide as the terminal, and `↑` `↓` walk down it half a screen at a time. `+` and `-` step the zoom. Small print in a scan becomes readable without leaving the terminal.
- **Three modes** on `F1` `F2` `F3`, or cycled with `m`: the page's text full width, the page as an image full width, or text on the left with the page on the right. The config picks which one a document opens in.
- **Edit the real document**: `e` opens the `.md` or `.tex` sitting beside the PDF, rebuilds it on save, and reloads the page. A small change is a real edit, not a patch painted over the page.
- **Corpus search**: `s` searches every indexed PDF, not just the open one, and opens the document on the page that carries the phrase.
- **Quote with a citation**: `y` copies the page's text with the file name and page number attached.
- **Reads scans too**: a scanned PDF has no text layer, so folio says so and shows you the page.
- **Remembers where you were**, per document, in plain text you can edit.
- **Zero idle cost**: nothing runs while you read. Text is extracted once per document, a page image rendered once per size, and both are kept.

## Install

Download the prebuilt binary from [Releases](https://github.com/isene/folio/releases), or build from source:

```bash
cargo build --release
cp target/release/folio ~/.local/bin/
```

Run `folio somefile.pdf`, or run it bare and pick up where you left off.

To make it the system PDF reader, so file managers and browsers hand PDFs to
it, install the desktop file that ships with this repo:

```bash
cp folio.desktop ~/.local/share/applications/
xdg-mime default folio.desktop application/pdf
```

It declares `Terminal=true`, which is how a TUI program advertises itself. A
terminal file manager that reads the desktop entry, [pointer](https://github.com/isene/pointer)
among them, then gives folio the terminal instead of launching it detached.

Needs `pdftotext` and `pdfinfo` (poppler-utils) for text, and `mutool` (mupdf-tools) for page images.

## Key Bindings

| Key | Action |
|-----|--------|
| `F1` `F2` `F3` | text / page / split |
| `m` `M` | cycle the modes forward / back |
| `j` `k` `↑` `↓` | scroll, turning the page at either end |
| `Space` `b` | next / previous page |
| `z` | in page mode: full width, and back to the whole page |
| `+` `-` | zoom in / out, between the whole page and full width |
| `gg` `G` | first / last page |
| `10g` | go to page 10 |
| `/` `n` `N` | find in this document, next match, previous |
| `s` | find across every indexed document |
| `e` | edit the source if there is one, else a text sidecar |
| `y` `Y` | yank this page with a citation / the document's path |
| `o` | open another document |
| `w` `W` | widen / narrow the text pane in split mode, as in pointer |
| `Ctrl-B` | borders: none, page pane, both, text pane |
| `Ctrl-W` | write the whole text beside the PDF, asking first if that file exists |
| `?` | help |
| `q` | quit |

## Editing and rebuilding

A PDF cannot be edited as text. It places each glyph at a fixed coordinate, and the font it carries usually holds only the characters the document already uses. So a replacement of a different length mis-spaces the line, and a character the font lacks cannot be typed at all.

What works is editing the source. If `notes.hl`, `paper.md` or `book.tex` sits beside the PDF, `e` opens that, and saving rebuilds the PDF and reloads the page. A HyperList is looked for first, since that is often the file the document was written in. In split mode you edit on the left and see the result on the right. The build commands are configurable, and default to `pandoc` for Markdown and `pdflatex` for LaTeX. `build_hl` is empty by default, since a HyperList has no one way to become a PDF: with nothing set, `e` saves your edit and leaves the PDF alone rather than half-rebuilding it.

With no source beside it, `e` gives you the extracted text in a `.txt` sidecar. That is a note about the document, not the document.

## Corpus search

```bash
folio --index            # indexes everything under `library` in the config
folio --index ~/Papers   # or one directory
```

Indexing extracts and caches the text of every PDF it finds. A directory of 39 documents takes about five seconds. Then `s` inside folio searches all of them and opens the one you pick.

## Files

- `~/.folio/config`: `mode`, `split`, `border`, `border_fg`, `editor`, `build_tex`, `build_md`, `build_hl`, `library`. All optional.
- `~/.folio/state`: where you were in each document, one tab-separated line each.
- `~/.folio/index`: the list of indexed documents.
- `~/.folio/cache/`: extracted text and rendered pages. Keyed by file and modification time, so a rebuilt PDF never shows a stale page. Safe to delete.

Example config:

```
mode = split
split = 55
border = 2
editor = scribe
library = /home/geir/Main
```

## License

Public domain (Unlicense). Created by [Geir Isene](https://isene.com).
