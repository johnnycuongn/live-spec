# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Project guidance lives in two files so every agent reads the same source:

- **AGENTS.md**: commands, invariants, cross-language contracts and decisions to preserve. The rules.
- **ARCHITECTURE.md**: how the app is built (layers, the live-analysis flow, sections, providers, persistence, security model, where to change things). The map. Read the relevant section before changing a flow.

@AGENTS.md

@ARCHITECTURE.md

## Claude Code specifics

- Live Spec itself drives Claude Code. When the app opens a project, `claude -p` runs with that project as its working directory, so the project's CLAUDE.md shapes Live Spec's notes and brief, including when Live Spec is opened on this repo. The non-Claude digest also inlines CLAUDE.md, AGENTS.md and ARCHITECTURE.md (`KEY_FILES` in `project/digest.rs`). Put new rules in AGENTS.md and new descriptions of how things work in ARCHITECTURE.md, rather than duplicating them here.
- To see the NDJSON that `ClaudeCli::stream` parses (useful when `parse_line` breaks after a CLI upgrade), run the same flags by hand:
  `echo "say hi" | claude -p --output-format stream-json --verbose --include-partial-messages --model haiku --max-turns 1`
