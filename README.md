# Collector → Lazer

**English** · [Русский](README.ru.md)

Install collections from [osucollector.com](https://osucollector.com) into **osu!lazer** — but only the star ratings you want.

A collection with 10 000 difficulties from 4★ to 8★, and you only want 7–8★? Pick those two bars, press one button. The app downloads the sets, removes every other difficulty from them and hands them to your running osu!lazer. Then one pass of lazer's setup wizard adds the collection itself.

## Download

Grab the latest release: **[Releases](https://github.com/X4vi3/osu-collector-lazer/releases/latest)** (Windows 10/11 x64).

- `…_x64-setup.exe` — installer. Updates itself from inside the app.
- `Collector-to-Lazer-portable.exe` — no install, just run it. New versions are downloaded manually.

Windows SmartScreen may warn about an unknown publisher: the app isn't code-signed (that costs money). Click **More info → Run anyway**.

## How to use

1. Paste a collection link (`https://osucollector.com/collections/12345/…`) or just its ID. You can also drop a `collection.db` file.
2. Click the star-rating bars you want. You'll see how many difficulties and sets that is, and how many osu!lazer already has.
3. Press **Download and install to Lazer** and keep osu!lazer open.
4. When it's done, follow the three steps on screen: in lazer, open **Settings → General → Run setup wizard → Import**, pick the folder the app shows, keep only **Collections** enabled and import.

Collections pile up in that folder, so you can install several and run the wizard once at the end. Re-importing is safe: lazer merges collections by name and never duplicates maps.

## What it does to your game

- **Never writes to lazer's database** (`client.realm`). Maps go in through lazer's own import (the same as double-clicking an `.osz`), collections through lazer's own wizard.
- **Reads** lazer's beatmap files (read-only) to skip difficulties you already have. Can be turned off in settings.
- lazer replaces a set when it is re-imported. The app accounts for that: difficulties you already have in a set are kept in the new archive, so nothing disappears.
- Remaining difficulties are copied byte-for-byte, so lazer matches them to the collection.

## Where maps come from

Sets are downloaded from public mirrors — [osu.direct](https://osu.direct), [catboy.best](https://catboy.best), [sayobot](https://osu.sayobot.cn), [nerinyan](https://nerinyan.moe) — several at once, respecting their rate limits. Video is never downloaded.

This is an unofficial project, not affiliated with osu!, osu!collector or any mirror. If you like osu!collector, consider supporting it with [their desktop client](https://osucollector.com/client).

## Building from source

Requirements: Node.js 20+, Rust (stable, MSVC), Visual Studio Build Tools, WebView2.

```powershell
npm ci
npm run tauri dev       # run in development
npm run tauri build     # build installer + exe into src-tauri/target/release
cargo test --manifest-path src-tauri/Cargo.toml
```

`npm run dev` / `npm run build` / `npm test` do the same through `scripts/env.ps1`, which can use a project-local toolchain in `.tools/`.

### Releasing

`scripts/release.ps1 0.2.0` bumps the version, tags and pushes; GitHub Actions builds the installer, the portable exe and the signed update manifest. The repository needs a `TAURI_SIGNING_PRIVATE_KEY` secret (the updater signing key).

## License

[MIT](LICENSE)
