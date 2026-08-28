# 1. Child-Process Companion Architecture

We decoupled Dewey's TUI library manager from reading (Continuum) and downloading (Labrador) by running companion tools as standalone child processes rather than embedding them directly. Dewey suspends the terminal raw mode, spawns Continuum with CLI arguments (`--file`, `--page`, `--mode`, `--storage-profile`), and captures structured JSON from stdout on exit to persist reading progress. This keeps the TUI lightweight, avoids linking heavy GUI libraries (GTK4) into the terminal binary, and enables independent development and lifecycle of each tool.
