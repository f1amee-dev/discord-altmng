# Discord Alt Manager

A lightweight desktop app for switching between Discord accounts without logging in and out every time.

Built with [Tauri](https://tauri.app/) (Rust backend) and React (TypeScript frontend).

## Download

If you just want to use the app, download the latest build from:

- [Latest release](https://github.com/f1amee-dev/discord-altmng/releases/latest)
- [All releases](https://github.com/f1amee-dev/discord-altmng/releases)

## What it does

- Save multiple Discord accounts with nicknames and color-coded avatars
- Switch between them in one click — the app swaps the auth token in Discord's local storage and relaunches it
- Supports Stable, PTB, and Canary channels on macOS and Windows
- Tokens are stored locally on your machine, nothing leaves your computer

## How it works

Discord stores its auth token in a Chromium LevelDB database. This app reads and writes to that database directly. When you "capture" an account, it grabs the token after you log in. When you "switch," it writes the saved token back and opens Discord.

## Getting started

You'll need [Node.js](https://nodejs.org/) and [Rust](https://rustup.rs/) installed.

```bash
npm install
npm run tauri dev
```

## Build releases

Build a release with:

```bash
npm run tauri build
```

For Windows releases, run the same command on a Windows machine.

## Project structure

```
src/            React frontend (TypeScript)
src-tauri/      Rust backend (Tauri)
  src/lib.rs    All the backend logic — profile CRUD, token management, Discord detection
```

## License

[MIT](LICENSE)
