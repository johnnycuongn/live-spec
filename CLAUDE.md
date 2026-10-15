# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

All project guidance (commands, architecture, cross-language contracts, invariants) lives in AGENTS.md so every agent reads the same source:

@AGENTS.md

## Claude Code specifics

- Live Spec itself drives Claude Code. When the app opens a project, `claude -p` runs with that project as its working directory, so the project's CLAUDE.md shapes Live Spec's notes and brief, including when Live Spec is opened on this repo. The non-Claude digest also inlines both CLAUDE.md and AGENTS.md. Put new guidance in AGENTS.md rather than duplicating it here.
- To see the NDJSON that `ClaudeCli::stream` parses (useful when `parse_line` breaks after a CLI upgrade), run the same flags by hand:
  `echo "say hi" | claude -p --output-format stream-json --verbose --include-partial-messages --model haiku --max-turns 1`
