---
name: slate-audio-volume
description: Change output volume, mute, or switch the audio output device with wpctl. Use for any request about sound volume, muting, or speakers/headphones. Linux with PipeWire only.
---

# Audio volume

Use `wpctl` (PipeWire). Do not open a settings GUI for this.

## Set volume

```
wpctl set-volume @DEFAULT_AUDIO_SINK@ 40%     # absolute
wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+     # relative
```

Tier: reversible. Undo: set the previous value, read first with `wpctl get-volume @DEFAULT_AUDIO_SINK@`.

## Mute / unmute

```
wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle
```

Tier: reversible.

## Switch output device

```
wpctl status            # find the sink id under "Sinks"
wpctl set-default <id>
```

Tier: reversible. Undo: set-default back to the previous id.

## Verify

`wpctl get-volume @DEFAULT_AUDIO_SINK@` prints the current volume and mute state.
