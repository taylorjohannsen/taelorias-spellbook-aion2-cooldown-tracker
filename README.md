# AION2 Auras

Cooldown overlay for AION 2 Global. Reads the game's network traffic through Npcap, like the DPS meters, and never touches the game client.

## Run

Needs [Npcap](https://npcap.com/#download) and [Rust](https://rustup.rs).

```sh
cargo run --release
```

Start it before logging in, or change zone once. Unlock it from the tray icon to move it.

`--demo` shows fake casts without the game. `--trace` logs every cast to `%APPDATA%\com.taylorjohannsen.aion2auras\auras.log`.

## Credits

Packet layouts from [RATmeter](https://github.com/Kuroukihime/AIon2-Dps-Meter) and [aion2-dps-meter](https://github.com/cyberbadger6969/aion2-dps-meter).
