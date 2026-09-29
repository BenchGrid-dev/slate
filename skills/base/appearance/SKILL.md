---
name: slate-appearance
description: Dark mode, light mode and the desktop background (wallpaper) on SlateOS. Use for any request like "dark mode", "light theme", "change the wallpaper", "use this picture as my background". SlateOS only.
---

# Appearance on SlateOS

One command does the whole desktop (windows, panel, terminals, notifications, launcher, GTK apps), live, and remembers the choice:

```
slate-theme dark
slate-theme light
slate-theme wallpaper /home/me/Pictures/sea.jpg     # PNG, JPEG or WebP; absolute path
slate-theme wallpaper default
slate-theme status                                 # {"theme": "...", "wallpaper": "..."}
```

Tier: reversible (the previous choice is one command away; say what it was).

Wallpaper requests: find the file first (`ls ~/Pictures`, `find ~ -iname '*.jpg'`); if the user names a picture that is not on disk, say so rather than guessing. Do not download images unless asked. A picture the user just took or downloaded is usually in `~/Pictures` or `~/Downloads`.

Verify with `slate-theme status` and, when the user wants to see it, a screenshot. The change is immediate; nothing needs a restart.
