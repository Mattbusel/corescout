# CoreScout with OpenAI Codex

`corescout report` and `corescout gate` read Codex's own session logs, the same way
they read Claude Code transcripts. What counts as a claim and what counts as a check is
the same for both; see [CLAIMS.md](CLAIMS.md).

## Where Codex keeps its sessions

Codex (the CLI, the IDE extension and the app) writes every session as a "rollout" of
JSON lines:

```
~/.codex/sessions/YYYY/MM/DD/rollout-<time>-<session id>.jsonl
~/.codex/archived_sessions/...        (sessions you archived)
```

`$CODEX_HOME` replaces `~/.codex` when it is set. `corescout report` reads both
folders where they are; unlike Claude Code's transcripts they are not copied into
CoreScout's data directory, because Codex does not delete them on a timer.

## `corescout report`

```
corescout report                    # Claude Code, Codex and Gemini CLI together
corescout report --source codex     # Codex only
corescout report --source codex --days 7 --card
corescout report --from <DIR>       # any folder of transcripts, any agent's format
```

With more than one agent installed, the report gives the share of unbacked claims for
each agent as well as overall, and names the agent on every claim it lists. `--json`
adds `sources`, `transcripts_by_source`, `by_source` and a `source` on each item in
`top_unbacked`.

For Codex a project is the working directory the session started in.

### What is read from a rollout

Codex has changed its rollout format several times, and all of them are read:

- **Current rollouts** record what you typed, what Codex said and every command it ran
  as completed items (`event_msg` lines with `item_completed`: `UserMessage`,
  `AgentMessage`, `CommandExecution`). A command's exit code and output come from its
  `CommandExecution` item.
- **2025 to mid-2026 rollouts** record the conversation as `user_message` and
  `agent_message` events, and the commands only as tool calls (`shell`,
  `shell_command`, `exec_command`, `local_shell_call`, or a code-mode `exec` script
  calling `tools.exec_command`) with the exit code in the tool output.
- **The first rollouts (spring 2025)** have no wrapper and no per-line timestamps; the
  session's start time stands in for every message.

A command you ran yourself with `!` is not Codex checking its work and is not counted.
A command that was still running when Codex moved on has no exit code and never backs
a claim. Instructions Codex injects (AGENTS.md, the environment, permissions) are not
turns.

## `corescout gate` as a Codex `Stop` hook

Codex runs `Stop` hooks with the same contract as Claude Code: the hook gets JSON on
stdin (`transcript_path`, `last_assistant_message`, `stop_hook_active`, ...), and exit
code 2 with a message on stderr keeps the turn going with that message as the next
instruction. `corescout gate` reads the rollout the payload names, so the same command
works for both.

Add it to `~/.codex/hooks.json`:

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          { "type": "command", "command": "corescout gate", "timeout": 30 }
        ]
      }
    ]
  }
}
```

or, equivalently, to `~/.codex/config.toml`:

```toml
[[hooks.Stop]]

[[hooks.Stop.hooks]]
type = "command"
command = "corescout gate"
timeout = 30
```

`timeout` is in seconds. For one repository, put the same `hooks.json` in that
repository's `.codex/` folder. If `corescout` is not on the `PATH` Codex runs hooks
with, use the full path to the program (on Windows a `commandWindows` entry next to
`command` overrides it there).

Then open `/hooks` in Codex and trust the hook. Codex runs a hook from your own
configuration only after you have reviewed it there, and asks again whenever the
hook's command changes. Hooks are on by default (`[features] hooks = true`).

### What the gate does in Codex

When Codex is about to end a turn, the gate looks at the final message (Codex sends it
in the payload) and at the commands Codex ran **in that turn**. If the message claims
success and no test, build or lint run passed, it exits 2 and tells Codex what it
claimed and what it last ran. Codex continues the turn and either runs the check or
says plainly that the change is unverified.

It lets the stop through when the message claims nothing, when a check passed in the
turn, when the stop already follows a block (`stop_hook_active`), and when it cannot
read the payload or the rollout (Codex sends no `transcript_path` for a session that
is not being recorded).

### Trying it by hand

```
corescout gate --transcript ~/.codex/sessions/2026/11/14/rollout-....jsonl
echo $?   # 2 means it would have blocked
```

`--source codex` reads the file as a Codex rollout without looking at its contents
first. `CORESCOUT_GATE_DEBUG=1` makes the gate say on stderr when it lets a stop
through.

## Limits

- The gate reads the rollout as it is on disk when the hook runs. Codex writes the
  rollout as it goes; a command whose completed item has not reached the file yet is
  not seen, which can only make the gate stricter, never let an unchecked claim
  through.
- Commands run through an MCP server or another tool that is not Codex's shell are not
  seen as checks.
