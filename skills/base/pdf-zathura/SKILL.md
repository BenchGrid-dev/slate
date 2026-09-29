---
name: slate-pdf-zathura
description: Read PDFs (and EPUB, DjVu) on SlateOS: open in zathura, go to a page, search, zoom, and extract text or metadata from the command line. Use for any request to open, read, search or summarise a PDF.
---

# PDFs with zathura

## Text first: do not open a window to read a PDF

```
pdftotext -layout file.pdf -          # the text of every page (poppler)
pdftotext -f 3 -l 3 file.pdf -        # one page
pdfinfo file.pdf                      # pages, title, author, size
```

If `pdftotext` is missing, `soffice --headless --convert-to txt file.pdf` also works. Tier: observe.

## Show it to the user

`desktop_launch` with `zathura file.pdf` (add `--page=12` to open at a page). zathura is keyboard-driven; with the window set, `desktop_key`:

| Want | Keys |
|---|---|
| next / previous page | `J` / `K` (or `n` / `p` for the next search hit) |
| go to page N | type `N` then `G` |
| zoom in / out / fit width | `+` / `-` / `s` |
| search | `/` then the text, Return |
| rotate | `r` |
| fullscreen | `F` |

Open it before clicking anything: zathura has no toolbar and no accessibility tree; screenshots are the way to verify what is shown.

Tier: observe. zathura never modifies the file.
