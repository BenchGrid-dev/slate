---
name: slate-printing
description: Print files, list printers, choose the default printer, and cancel print jobs with CUPS (lp, lpstat, cancel). Use for any request to print something or about printers and the print queue. Linux with CUPS only.
---

# Printing

Use the CUPS command-line tools. Printers on the network that support driverless printing (IPP Everywhere, AirPrint; most made in the last ten years) are usable without being added: they appear in `lpstat -e` on their own. A USB printer, a printer at a fixed address, or an older one that needs a driver is added by the person in Settings, Printers, Add a Printer (`slate-settings printers`).

## See the printers

```
lpstat -e                 # every printer you can print to, network ones included
lpstat -d                 # the default one
lpstat -p                 # the added ones and what they are doing
```

Tier: observe.

## Print

```
lp -d <printer> <file>                          # PDF, images and plain text print directly
lp -d <printer> -n 2 -o sides=two-sided-long-edge <file>
lp -d <printer> -o media=A4 -o page-ranges=1-3 <file>
```

Tier: confirm. Paper and ink cannot be given back: say which printer, how many copies and which pages before printing. Office documents: convert to PDF first (see the office-libreoffice skill) and print the PDF.

## The queue

```
lpstat -o                 # jobs waiting or printing
cancel <job-id>           # e.g. cancel Office_Laser-12
```

Tier: reversible (a cancelled job can be sent again).

## Default printer

```
lpoptions -d <printer>
```

Tier: reversible.

## Verify

`lpstat -W completed -o <printer>` lists the job once the printer took it.
