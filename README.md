# Oversight

AI coding agents are generic. Your environment isn't.

Why waste time trying to fix things manually when an AI can do it for you?

## Installation

Simply clone the repository and run
```
make install
```
or
```
./install.sh
```
 Works on macOS and Linux.

## Why?

Agents have to rediscover your environment every single time they run. Even if you add things to your `agents.md`, the file eventually becomes bloated and the agent ignores directions. Even if agents commit things to their "memories", their summaries are often overcomplicated, context-heavy, and often pointless with fast-moving software development.

Oversight presents an easy way for agents to access a wiki about your system. A wiki that's been constructed through every interaction you've had with agents on your machine, processed to find learnings about your preferences and quirks about your setup.

## How to help out

Oversight relies on *providers* that allow it to process conversations through different modalities. It already supports:
- Claude
- Codex
- Gemini
- OpenCode
- Crush

If you use a different provider, feel free to open a pull request for it!

## What's next?

KB maintenance, à la Claude dreams.
