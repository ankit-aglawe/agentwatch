# Recording the agentwatch demo

We use [VHS](https://github.com/charmbracelet/vhs) — a declarative,
reproducible terminal recorder. Re-runs against the latest TUI without
manual re-takes.

## One-time install

```sh
# macOS
brew install vhs

# Linux (Go required)
go install github.com/charmbracelet/vhs@latest

# Optional for MP4/WebM output
brew install ffmpeg     # or: apt install ffmpeg
```

VHS also needs `ttyd` (auto-installed by Homebrew on macOS; `apt install
ttyd` on Linux).

## Record

```sh
# from the repo root
cargo install --path crates/agentwatch-cli      # so `agentwatch` is on PATH
vhs vhs/demo.tape
```

Outputs land in `assets/`:
- `demo.gif` — for README + Twitter
- `demo.mp4` — for HN / longer-form
- `demo.webm` — for landing page

## Why VHS over alternatives

| Tool | Verdict |
|---|---|
| **VHS** | ✅ chosen — declarative `.tape` file, GIF + MP4 + WebM, reproducible |
| asciinema | great for "play in browser" but no embeddable GIF |
| terminalizer | older, npm-based, fragile |
| t-rec | screen-capture style, no scripted keystrokes |
| OBS | manual, not reproducible |

## Tape file tips

- `Sleep` is the lever. Long pauses = viewers can read; short pauses = momentum.
- `Set PlaybackSpeed` is post-record speedup. We keep it at 1.0 because the
  TUI already moves; if a viewer needs slower, they can pause the GIF.
- Re-record after any TUI layout change. Don't ship a stale GIF.
