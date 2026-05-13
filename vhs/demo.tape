# agentwatch — VHS recording script
# https://github.com/charmbracelet/vhs
#
# Run:    vhs vhs/demo.tape
# Output: assets/demo.gif (+ assets/demo.mp4 if ffmpeg installed)

Output assets/demo.gif
Output assets/demo.mp4
Output assets/demo.webm

Set Shell "zsh"
Set FontFamily "JetBrains Mono"
Set FontSize 14
Set Width 1440
Set Height 860
Set Padding 28
Set Theme "Catppuccin Mocha"
Set TypingSpeed 35ms
Set PlaybackSpeed 1.0
Set LoopOffset 1

# --- Opener: banner -------------------------------------------------
Type "agentwatch banner"
Sleep 600ms
Enter
Sleep 1500ms

# --- The pitch ------------------------------------------------------
Type "# htop for your AI coding agents. Local. Free. Zero config."
Sleep 1200ms
Ctrl+U

# --- Demo data + live TUI -------------------------------------------
Type "agentwatch demo --seed 42"
Sleep 500ms
Enter

# Sit on the TUI long enough for the eye to scan the panels.
# Order viewers will see: live banner -> agent bars -> today summary ->
# sessions table -> charts -> hot projects/files.
Sleep 9s

# Optional: hit a key the user would actually press, e.g. to scroll or
# refresh. Keep it minimal — the chart is the star.
Sleep 2s

# Clean exit so the loop is clean (LoopOffset 1 keeps the title slide).
Type "q"
Sleep 600ms
