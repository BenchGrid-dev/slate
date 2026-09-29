---
name: slate-office-libreoffice
description: Documents, spreadsheets and presentations with LibreOffice on SlateOS: create, open, edit, convert (to PDF, DOCX, XLSX, CSV…) and export. Use for any request about Word/Excel/PowerPoint-style files, letters, reports, invoices, slides.
---

# Office documents with LibreOffice

LibreOffice is installed as `soffice` (Writer, Calc, Impress, Draw). Two paths, cheapest first.

## 1. Headless, no window: conversions and batch work

```
soffice --headless --convert-to pdf --outdir ~/Documents report.docx     # any format → PDF
soffice --headless --convert-to docx letter.odt                          # ODF ↔ Office formats
soffice --headless --convert-to csv:"Text - txt - csv (StarCalc)":44,34,76 data.xlsx
soffice --headless --convert-to xlsx table.csv
```

Tier: reversible (writes new files next to the source or in `--outdir`; never overwrites the source). Verify with `ls -l` and, for PDFs, `pdfinfo` if present.

To create a document from text without a GUI, write Markdown or HTML and convert it: `soffice --headless --convert-to odt draft.html` (headings, lists, tables and bold survive; images by absolute path).

## 2. In a window: editing, formatting, anything the user wants to see

- Open: `desktop_launch` with `soffice` and the file (`soffice --writer`, `--calc`, `--impress` for a new document of that kind).
- LibreOffice exposes a complete accessibility tree. `desktop_elements` lists the toolbar buttons, menu items, dialog fields and, for Writer, the paragraphs; `desktop_read` returns the document text. Prefer `desktop_element_click` and `desktop_element_set_text` over pointer clicks and typing.
- Menus: click the menu (`File`, `Format`…) as an element, then call `desktop_elements` again for the items that appeared. Dialogs (Save As, Export as PDF) are new windows: `desktop_windows` shows them; their fields and buttons are elements.
- Typing prose into Writer: `desktop_element_click` the document area, then `desktop_type` with the window set. Cell values in Calc: click the cell, type, press Return.
- Save: Ctrl+S with the window set (`desktop_key`), or File → Save as elements. Saving in an Office format shows a "Keep current format" dialog: choose the format the user asked for.

Tier: reversible for edits to files under the snapshot root; Confirm when overwriting a file the user did not name.

## Verify

Reopen or convert the result (`soffice --headless --convert-to txt` on a document gives its text) and read it back before reporting.
