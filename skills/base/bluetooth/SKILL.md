---
name: slate-bluetooth
description: Turn Bluetooth on or off, list, connect, disconnect and forget Bluetooth devices (headphones, speakers, mice, keyboards, phones) with bluetoothctl. Use for any request about Bluetooth or wireless audio and input devices. Linux with BlueZ only.
---

# Bluetooth

Use `bluetoothctl` (BlueZ) for devices that are already paired. Pairing a new device is the person's job: it often shows a code they must compare or type on the device, and the device must be put in pairing mode by hand. For that, open Settings on the Bluetooth page (`slate-settings bluetooth`) and tell them what to do; Settings answers the pairing request.

## See what there is

```
bluetoothctl show                     # the adapter: Powered, Discoverable
bluetoothctl devices Paired           # MAC and name of every paired device
bluetoothctl devices Connected
bluetoothctl info <MAC>               # Connected, Trusted, battery level if it reports one
```

Tier: observe. No adapter in `show`: the computer has no Bluetooth or lacks its firmware; say so.

## Connect / disconnect a paired device

```
bluetoothctl connect <MAC>
bluetoothctl disconnect <MAC>
```

Tier: reversible. Headphones become the sound output when they connect; `wpctl status` shows it (see the audio-volume skill).

## Power

```
bluetoothctl power on       # or off
```

Tier: reversible. If it fails with "Blocked", run `rfkill unblock bluetooth` first.

## Forget a device

```
bluetoothctl remove <MAC>
```

Tier: confirm. Using it again means pairing it again, by the person.

## Verify

`bluetoothctl info <MAC>` shows `Connected: yes` (or `no` after a disconnect).
