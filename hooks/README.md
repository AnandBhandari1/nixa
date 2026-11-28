# Nixa status hooks

Each agent reports its own status to the terminal it runs in, with `nixa term status working|blocked|done|idle|error`.
The hooks do nothing outside a Nixa terminal: they need NIXA_TERM_ID, NIXA_ENGINE_SOCKET and TERM_PROGRAM=nixa.
They call the path in NIXA_BIN (nixa-engine sets it in every terminal to the nixa command next to it), else the nixa command near the hooks folder: Contents/MacOS/nixa in the app, or target/gpui/debug or target/gpui/release in a source folder.
They never call the nixa command on PATH.

Build the command first: `CARGO_TARGET_DIR=target/gpui cargo build -p nixa-cli`

## Install with nixa hooks

- See what is there: `target/gpui/debug/nixa hooks status`
- See the exact change, nothing is written: `target/gpui/debug/nixa hooks install`
- Write it: `target/gpui/debug/nixa hooks install --apply`
- Take the hooks out again: `target/gpui/debug/nixa hooks remove --apply`

Pick agents by name: `claude`, `codex`, `omp`, `cursor`, `agy` (default: all five).
`--home DIR` uses another home folder, for tests. `--hooks DIR` uses another hooks folder. The default is the hooks folder near the nixa command: Contents/Resources/hooks in the app, hooks/ in a source folder.
The JSON files here are templates. Install writes the full path of nixa-status.sh into each hook, so no fixed path is kept in this folder.
With `--apply`, a changed file is first copied to `NAME.nixa-backup-DATE` next to it.
The Nixa hooks are merged into the file: other hooks and settings stay. Nixa hooks are found by the path of nixa-status.sh in this hooks folder.

- Claude Code: `~/.claude/settings.json` (or CLAUDE_CONFIG_DIR). Check with /hooks inside Claude Code.
- Codex: `~/.codex/hooks.json` (or CODEX_HOME). Then open Codex, run /hooks and trust the new hooks. Codex skips hooks that are not trusted.
- omp: a link `~/.omp/agent/extensions/nixa-status.ts` to `hooks/omp/nixa-status.ts`. A link, not a copy, so the extension finds the nixa command near the hooks folder.
- Cursor: `~/.cursor/hooks.json`, used by the Cursor editor and by cursor-agent. Install adds `"version": 1` when the file has none. Restart Cursor and cursor-agent after a change.
- Antigravity (agy): the `nixa-status` group in `~/.gemini/config/hooks.json`. Other groups stay as they are. Start agy again after a change.

## Without an install

Terminals that Nixa starts get some hooks with no install:
- Claude Code gets all its status hooks with `--settings`.
- omp gets the extension with `-e hooks/omp/nixa-status.ts`, unless the link in `~/.omp/agent/extensions` is there or the first word is a subcommand.
- Codex gets only a done notify with `-c notify=...`. Codex skips hooks it was not told to trust, so working and needs you need the install and /hooks.
- agy has no per-run hooks, so it needs the install.

## Resume

The SessionStart and UserPromptSubmit hooks of Claude Code and Codex, the sessionStart and beforeSubmitPrompt hooks of Cursor, and the SessionStart and PreInvocation hooks of agy (conversationId), run `nixa-status.sh --resume AGENT STATE`. The script gives the hook JSON to `nixa hooks session AGENT`, which reads its session_id (Cursor: conversation_id) and sets two labels on the terminal: `resume` (for example `claude --resume <session id>`) and `resume.pid` (the terminal's process id).
The omp extension does the same with `nixa hooks session omp --id <session id>` on session_start, session_switch and input.
When the agent stops without a clean exit, Nixa offers "resume in a new tab" with that command. A clean exit (code 0) forgets the labels.
The permission checks off flag is added only when the agent runs with it: the terminal was started with the flag, or Claude Code reports permission_mode bypassPermissions.
Labels are written only when they change.

## Events

- Claude Code: SessionStart idle, UserPromptSubmit working, PreToolUse working, PostToolUse working, PostToolUseFailure working, PermissionRequest blocked, Notification (permission_prompt, elicitation_dialog) blocked, Stop done, StopFailure error, SessionEnd idle.
- Codex: SessionStart idle, UserPromptSubmit working, PreToolUse working, PostToolUse working, PermissionRequest blocked, Stop done, Interrupt idle, SessionEnd idle.
- Cursor: sessionStart idle, beforeSubmitPrompt working, postToolUse working, afterShellExecution working, afterMCPExecution working, afterFileEdit working, stop done, sessionEnd idle. Cursor has no hook event for a permission question, so Cursor never shows "needs you" from a hook.
- Antigravity: SessionStart idle, PreInvocation working, PostToolUse working, Stop done. agy has no hook event for a permission question.
- omp: session_start idle, input working, agent_start working, tool_approval_requested blocked, tool_approval_resolved working, plan_review_requested blocked, plan_review_resolved working, agent_end done, session_shutdown idle.
