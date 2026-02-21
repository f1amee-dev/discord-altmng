# Contributing

Thanks for wanting to help out! Here's how to get going.

## Setup

1. Clone the repo
2. Install [Node.js](https://nodejs.org/) and [Rust](https://rustup.rs/) if you haven't already
3. Run `npm install` to grab the frontend dependencies
4. Run `npm run tauri dev` to start the app in dev mode

## Making changes

- Create a branch off `main` for your work
- Keep commits focused — one thing per commit is ideal
- Test on your machine before opening a PR (we don't have CI yet, so manual testing matters)

## What could use help

- Linux support (the backend currently only handles macOS and Windows)
- Better error messages when Discord isn't installed
- UI improvements and accessibility
- Tests — there aren't any yet, and that's not great

## Code style

Nothing fancy. Just keep it consistent with what's already there. The frontend is TypeScript + React, the backend is Rust. If the compiler and `tsc` are happy, we're probably good.

## Pull requests

Open a PR against `main` with a short description of what you changed and why. Screenshots are nice if you touched the UI.
