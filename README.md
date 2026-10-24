# Battery Tray

A small Windows tray app (Rust) that shows the battery level of **Razer mice**
and **SteelSeries Arctis headsets** side by side, without Synapse or GG.

- The tray icon shows the **lowest** battery level of all connected devices,
  coloured green (≥ 50 %), amber (20–49 %) or red (< 20 %). A yellow corner means
  charging. Grey means every device is asleep or off; it then shows the last value seen.
- Hover for every device's level; right-click for the full list, *Refresh now*,
  *Start with Windows* and *Quit*. Left-click refreshes.
- Devices are polled every 60 s (`battery-tray.exe --interval 30` to change it).

## Files

| File | What it is |
|---|---|
| `battery-tray.exe` | The tray app (no console window). |
| `battery-cli.exe`  | Console read-out for checking support and debugging. |

```
battery-cli.exe            one line per device
battery-cli.exe --json     machine-readable
battery-cli.exe --debug    also lists Razer/SteelSeries HID collections and raw traffic
battery-cli.exe --models   every supported model with its USB id
```

## Supported devices

- **68 Razer mice** with batteries: everything OpenRazer can read a charge level from
  (DeathAdder V2/V3/V4 Pro, Viper Ultimate / V2 Pro / V3 Pro, Basilisk Ultimate / V3 Pro,
  Naga Pro, Cobra Pro, Pro Click, Orochi V2, HyperSpeed models and more), wired and
  over the 2.4 GHz receiver.
- **36 SteelSeries Arctis variants**: Arctis 1 / 7 / 7X / 7P / 9 / Pro (2019) / Pro Wireless,
  Arctis 7+ family, Nova 7 / 7X / 7P (old and Jan-2026 firmware), Nova 5 / 5X,
  Nova 3P / 3X Wireless, Nova Pro Wireless (base station) and GameBuds.

Run `battery-cli.exe --models` for the exact list with USB ids.

## How it works

- **Razer**: sends Razer's 90-byte HID feature report (command class `0x07`,
  command `0x80` = battery, `0x84` = charging) with the model's transaction id,
  waits the model's response delay and reads the answer back. Level is `0–255`,
  scaled to percent. Windows lets apps send feature reports to the mouse's HID
  collection even though it owns the mouse input.
- **SteelSeries**: writes the model's status request (e.g. `00 B0` for the Nova
  family) to the right HID collection and parses the reply (percentage, 0–4 or
  0–8 steps depending on the model).

A mouse that is asleep answers "timeout" (or a level of zero), so it shows as
asleep with its last known value rather than 0 %. Arctis readings only accept
replies that are status reports, so chat-mix or volume events on the same
interface can't be misread as a battery value.

## Notes

- Synapse or SteelSeries GG can stay installed. If readings fail while they run,
  quit them and compare.
- When the cable and the receiver are both connected, a Razer mouse appears
  once (the copy that answers wins).
- A model that isn't listed won't show up. To add one, find its USB id with
  `battery-cli.exe --debug` and add a row to `src/razer_devices.rs` or
  `ARCTIS_MODELS` in `src/steelseries.rs`.

## Building from source

On Windows with Rust installed (https://rustup.rs):

```
cargo build --release
```

The executables end up in `target\release\`. Unit tests: `cargo test`.

`tools/gen_razer.py` regenerates `src/razer_devices.rs` from an OpenRazer checkout:
`python tools/gen_razer.py path\to\openrazer src\razer_devices.rs`.

## Credits

The device protocols come from two open-source projects:

- [OpenRazer](https://github.com/openrazer/openrazer) (GPL-2.0): Razer report
  format, transaction ids, response delays and product ids.
- [HeadsetControl](https://github.com/Sapd/HeadsetControl) (GPL-3.0): SteelSeries
  Arctis request bytes, response layouts and product ids.

No code is copied from either project, but because the device tables are derived
from them, GPL-3.0 is the safe license if you publish this.
