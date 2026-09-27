---
name: slate-network-wifi
description: List, connect to, or disconnect from Wi-Fi with nmcli. Use for any request about Wi-Fi, wireless networks, or being offline. Linux with NetworkManager only.
---

# Wi-Fi

Use `nmcli` (NetworkManager). Do not open a settings GUI for this.

## See what is around and what is connected

```
nmcli device status
nmcli device wifi list
nmcli connection show --active
```

Tier: observe.

## Connect

```
nmcli device wifi connect "<SSID>" password "<password>"
```

Tier: confirm. Connecting changes which network the user's traffic goes through, and the password is a credential: never guess it, never echo it back, ask the user for it if it is not already saved (`nmcli connection show` lists saved profiles; `nmcli connection up "<name>"` uses one).

## Disconnect / forget

```
nmcli device disconnect wlan0
nmcli connection delete "<name>"     # tier: confirm, this forgets the password
```

## Verify

`nmcli device status` shows the device as `connected`, and `ping -c 1 1.1.1.1` succeeds.
