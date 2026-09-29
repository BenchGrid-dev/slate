---
name: slate-media-mpv
description: Play music and video on SlateOS with mpv (files, folders, streams and URLs), pause/seek/volume during playback, and inspect media files. Use for any request to play, pause, skip, or ask about a video or audio file.
---

# Media playback with mpv

## Play

```
mpv --input-ipc-server=/tmp/mpv-slate ~/Music/album/          # a folder is a playlist
mpv --input-ipc-server=/tmp/mpv-slate --no-video song.flac      # audio only
mpv --input-ipc-server=/tmp/mpv-slate "https://…"               # streams and sites via yt-dlp if installed
```

Always start it through `desktop_launch` with `--input-ipc-server=/tmp/mpv-slate` so it can be controlled without keys. Tier: observe (playback changes nothing on disk).

## Control a running player

Send JSON commands to the socket (needs `socat` or a small Python snippet):

```
echo '{"command":["set_property","pause",true]}'  | socat - /tmp/mpv-slate     # pause
echo '{"command":["set_property","pause",false]}' | socat - /tmp/mpv-slate     # resume
echo '{"command":["seek",30]}'                    | socat - /tmp/mpv-slate     # +30 s
echo '{"command":["playlist-next"]}'              | socat - /tmp/mpv-slate     # next track
echo '{"command":["set_property","volume",60]}'   | socat - /tmp/mpv-slate     # player volume (system volume: the audio-volume skill)
echo '{"command":["get_property","time-pos"]}'    | socat - /tmp/mpv-slate     # where are we
echo '{"command":["quit"]}'                       | socat - /tmp/mpv-slate
```

Without socat: `python3 -c 'import socket,json;s=socket.socket(socket.AF_UNIX);s.connect("/tmp/mpv-slate");s.send(json.dumps({"command":["set_property","pause",True]}).encode()+b"\n")'`. Keys with the window set also work (`space` pause, `Left`/`Right` seek, `q` quit), but the socket is exact and needs no focus.

## Inspect a file

`mpv --no-config --frames=0 --term-playing-msg='${duration} ${width}x${height} ${audio-codec-name}' file` prints duration and codecs, or `ffprobe file` if ffmpeg is installed.

## Verify

`get_property pause` / `time-pos` over the socket tells you the real state; report that.
