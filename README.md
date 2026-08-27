# folio

<img src="img/folio.svg" align="right" width="150">

**The terminal PDF reader. Written in Rust.**

![Rust](https://img.shields.io/badge/language-Rust-f74c00) ![License](https://img.shields.io/badge/license-Unlicense-green) ![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS-blue) ![Stay Amazing](https://img.shields.io/badge/Stay-Amazing-important)

A PDF is two documents in one: the words, and the page they were set on. Most readers show only the second. folio shows either, or both at once, because reading a paper and quoting from it want different things. Built on [Crust](https://github.com/isene/crust) and [Glow](https://github.com/isene/glow), part of the [Fe2O3 suite](https://github.com/isene/fe2o3).

## Features

- **Three modes**: the page's text full width, the page as an image full width, or text on the left with the page on the right. The config picks which one a document opens in.
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

Needs `pdftotext` and `pdfinfo` (poppler-utils) for text, and `mutool` (mupdf-tools) for page images.

## Key Bindings

| Key | Action |
|-----|--------|
| `1` `2` `3` | text / page / split |
| `m` | cycle the modes |
| `j` `k` `↑` `↓` | scroll, turning the page at either end |
| `Space` `b` | next / previous page |
| `g` `G` | first / last page |
| `/` `n` `N` | find in this document, next match, previous |
| `s` | find across every indexed document |
| `e` | edit the source if there is one, else a text sidecar |
| `y` | yank this page's text with a citation |
| `w` | write the whole text beside the PDF |
| `+` `-` | widen / narrow the text pane in split mode |
| `?` | help |
| `q` | quit |

## Editing and rebuilding

A PDF cannot be edited as text. It places each glyph at a fixed coordinate, and the font it carries usually holds only the characters the document already uses. So a replacement of a different length mis-spaces the line, and a character the font lacks cannot be typed at all.

What works is editing the source. If `paper.md` or `book.tex` sits beside `paper.pdf`, `e` opens that, and saving rebuilds the PDF and reloads the page. In split mode you edit on the left and see the result on the right. The build commands are configurable, and default to `pandoc` for Markdown and `pdflatex` for LaTeX.

With no source beside it, `e` gives you the extracted text in a `.txt` sidecar. That is a note about the document, not the document.

## Corpus search

```bash
folio --index            # indexes everything under `library` in the config
folio --index ~/Papers   # or one directory
```

Indexing extracts and caches the text of every PDF it finds. A directory of 39 documents takes about five seconds. Then `s` inside folio searches all of them and opens the one you pick.

## Files

- `~/.folio/config`: `mode`, `split`, `editor`, `build_tex`, `build_md`, `library`. All optional.
- `~/.folio/state`: where you were in each document, one tab-separated line each.
- `~/.folio/index`: the list of indexed documents.
- `~/.folio/cache/`: extracted text and rendered pages. Keyed by file and modification time, so a rebuilt PDF never shows a stale page. Safe to delete.

Example config:

```
mode = split
split = 55
editor = scribe
library = /home/geir/Main
```

## License

Public domain (Unlicense). Created by [Geir Isene](https://isene.com).
