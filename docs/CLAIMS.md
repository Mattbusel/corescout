# Did the AI check before it said "done"?

Two commands answer that for Claude Code, OpenAI Codex and Gemini CLI, from the
transcripts they already write:

| Agent | Transcripts |
| --- | --- |
| Claude Code | `~/.claude/projects/**/*.jsonl` |
| Codex | `~/.codex/sessions/**/*.jsonl` and `~/.codex/archived_sessions` |
| Gemini CLI | `~/.gemini/tmp/<project hash>/chats/session-*.jsonl` (`.json` in older versions) |

Neither needs the CoreScout service running, and neither sends anything anywhere.

- `corescout report` looks back over your history.
- `corescout gate` is a `Stop` hook for Claude Code and Codex, and an `AfterAgent`
  hook for Gemini CLI, that stops the agent from ending a turn on an unchecked "done".

Each agent's format is read into the same model (a turn, the commands run in it with
their exit codes, the text the assistant wrote), so a claim and a check mean the same
thing whichever agent made them. Codex has its own page: [CODEX.md](CODEX.md).

Cursor is not read. Its hooks pass a `transcript_path`, but Cursor does not document
the transcript's format, and a reader built by guessing at one version's files would
break silently on the next.

## What counts as a claim, and what counts as a check

A **claim** is an assistant message that says the work succeeded: "tests pass",
"all green", "build passes", "deployed", "fixed", "done", and close variants
("tests passed", "Tests are green", "I've fixed the crash", "The build succeeded").
Questions, negations and conditionals are not claims ("Is it done?", "The tests don't
pass yet", "It is not deployed", "Once the tests pass I will push"), and neither is
text inside a code block. A negation only takes a claim back when it comes before it
in the same clause; a condition, a plan or a failure ("if", "when", "will", "failed")
when it comes anywhere before it. After the claim those words are qualifiers, so
"All 52 tests pass, 0 failed.", "All tests pass with no warnings." and "Fixed the crash
when input is empty." are claims. What does take a claim back after it is an
admission that it was not checked: "Done, but I haven't run the tests", "not tested
yet", "unverified". "Fixed" next to money or size words ("prices are fixed for the
term", "a fixed fee") and "done" after "work", "be" or "get" ("work done", "to be
done") are left out too.

Only the **last** claim in a turn counts, because that is the one you read. A *turn*
runs from something you typed to the next thing you type.

A **check** is a shell command the agent ran (Claude Code's `Bash` or `PowerShell`
tool, Codex's shell, Gemini CLI's `run_shell_command`) that runs tests, a build or a
linter, for example `cargo test`, `cargo clippy`, `npm test`, `npm run build`,
`pnpm lint`, `npx tsc`, `pytest`, `python -m unittest`, `go test`, `make`,
`./gradlew test`, `dotnet build`, `ruff check`, `eslint`. `cd app && npm test` counts;
`echo cargo test` does not. A check **passed** when the agent recorded exit code 0
and the output carries no unambiguous failure marker (`test result: FAILED`,
`error: could not compile`, `npm ERR!`, ...). That second rule exists because
`cargo test | tail` exits with `tail`'s code. A call that a hook refused, that was
interrupted or that was sent to the background has no exit code and is not a pass, and
a command you ran yourself (Codex's `!cmd`) is not the agent checking at all.

## `corescout report`

```
corescout report                 # the last 30 days
corescout report --days 7        # the last week
corescout report --top 25        # list 25 recent unbacked claims instead of 10
corescout report --card          # a one-screen card you can paste anywhere
corescout report --json          # everything, for a program
corescout report --no-store      # read in place, copy nothing
corescout report --from <DIR>    # only the transcripts in DIR, read in place
corescout report --source codex  # one agent: claude, codex, gemini, or all (the default)
```

It prints the share of success claims in the window that had **no passing check in
that turn or in the turn just before it**, then the most recent unbacked claims (ten,
or `--top <N>`), each with its time (UTC), session, project folder, the sentence that
made the claim, and the last command that ran before it with its exit code. With more
than one agent installed it also gives each agent's share and names the agent on each
claim it lists; the header names the agents whose transcripts it found.

`--card` keeps only counts and the kinds of claim: no paths, session ids, project names
or message text. `--json` returns `claims`, `unbacked`, `unbacked_percent`,
`sessions`, `projects`, `by_phrase` and `top_unbacked` (each with `source`, `session`,
`project`, `timestamp`, `claim`, `phrase`, `last_command`, `last_exit`), and per agent
`sources` (the agents with a transcript read), `sources_searched`,
`transcripts_by_source` and `by_source` (`source`, `claims`, `unbacked`,
`unbacked_percent`). For Codex a project is the session's working directory; for
Gemini CLI it is Gemini's project hash.

`--from <DIR>` reads only the transcripts under `DIR`, where they are. It never copies
them into the store and never reads the store, so a report on another folder is a
report on that folder alone and cannot change the history kept for
`~/.claude/projects`. Every `.jsonl` file in it, and every `.json` file that is a
Gemini CLI session, is read in the format its contents show, so one folder can hold
transcripts from all three agents; `--source` still narrows which are counted.

A `--days` value larger than your history simply means "everything".

`$CLAUDE_CONFIG_DIR`, `$CODEX_HOME` and `$GEMINI_CLI_HOME` move the folders the
report looks in, as they move the agents' own.

### Your history outlives Claude Code's cleanup

Claude Code deletes transcripts older than 30 days by default (`cleanupPeriodDays`).
So each run of `corescout report` first copies any transcript that is new or has grown
into CoreScout's data directory, under `claude-transcripts/`, and then reads from
there. That is `%LOCALAPPDATA%\CoreScout\claude-transcripts` on Windows and
`~/.local/share/corescout/claude-transcripts` elsewhere (or `$CORESCOUT_DATA_DIR`).
Deleting that folder deletes the copy. `--no-store` skips the copy and reads the
transcripts where they are.

The first run copies **all** of `~/.claude/projects`, whatever `--days` says, because
keeping the history is the point. That can be gigabytes if you use Claude Code a lot,
so before it starts the report says how many transcripts and how many MB it is about to
copy, and it prints progress as it goes. Later runs copy only what is new or has grown,
and say so again only when that is large. A copy is written beside the old one and
renamed over it, so an interrupted run never leaves a half-written file in place of a
good copy. A transcript that cannot be copied (a locked file, a full disk) is skipped
with a note on stderr and read where it is, so one bad file never stops the report.

The Privacy page (`corescout privacy`, and Settings > Privacy in the app) lists the
copies with their count and size, and its byte total includes them. Forgetting your AI
history does not delete them; deleting the folder does.

## `corescout gate`: the Stop hook

When the agent is about to end a turn, the gate reads the transcript. If the final
assistant message claims success and **no check passed in that turn**, it exits 2 and
writes a message to stderr. The agent shows the message to the model and keeps the
turn going, so it runs the check or says plainly that the change is unverified. It
tells the transcript's format from its contents; `--source claude|codex|gemini` forces
one.

It lets the stop through (exit 0) when:

- the final message claims nothing,
- a check passed in the turn,
- the stop is already the result of a block (`stop_hook_active` is true), so it can
  never loop, or
- it cannot read the hook payload or the transcript. A gate that breaks your session
  because of its own bug is a gate you would uninstall.

### Setting it up in Claude Code

Add this to `~/.claude/settings.json` (every project) or `.claude/settings.json` in
one repository:

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          { "type": "command", "command": "corescout gate" }
        ]
      }
    ]
  }
}
```

If `corescout` is not on the `PATH` Claude Code runs hooks with, use the full path to
the program instead. If you already have a `Stop` entry, add the
`{ "type": "command", "command": "corescout gate" }` object to its `hooks` list.

### Setting it up in Codex

See [CODEX.md](CODEX.md#corescout-gate-as-a-codex-stop-hook): the same command, in
`~/.codex/hooks.json` or `config.toml` under `Stop`, then trusted once in `/hooks`.

### Setting it up in Gemini CLI

Gemini CLI's `AfterAgent` hook runs when the model has written its final response, and
exit code 2 rejects the response and retries with the message on stderr as the next
prompt. The payload carries `transcript_path`, the response (`prompt_response`) and
`stop_hook_active`, so the gate works unchanged. Add this to `~/.gemini/settings.json`
(or `.gemini/settings.json` in one project):

```json
{
  "hooks": {
    "AfterAgent": [
      {
        "hooks": [
          {
            "name": "corescout-gate",
            "type": "command",
            "command": "corescout gate",
            "timeout": 30000
          }
        ]
      }
    ]
  }
}
```

`timeout` is in milliseconds here. `/hooks panel` in Gemini CLI lists it. A project
hook is treated as new and untrusted until you approve it, and again whenever it
changes.

### Trying it by hand

```
corescout gate --transcript ~/.claude/projects/<project>/<session>.jsonl
echo $?   # 2 means it would have blocked
```

`CORESCOUT_GATE_DEBUG=1` makes it say on stderr when it lets a stop through.

## Limits

- It reads words, not meaning. "Done" about a document, an email or a purchase counts
  as a claim, and nothing short of tests could back it. Read the percentage as "how
  often the assistant said it was finished without running a check", not as "how often
  it was wrong".
- A check that passed is not proof that it tested the change. `cargo test` on a
  package the edit did not touch still counts.
- The report credits a check in the turn just before the claim, so "run the tests"
  followed by "is it working?" / "yes, it's done" is backed. The gate only credits the
  turn it is stopping.
- Only shell tool calls are read as checks: `Bash` and `PowerShell` in Claude Code,
  Codex's shell, Gemini CLI's `run_shell_command`. A test run through some other tool
  (an MCP server, a background task read later) is not seen.
- Gemini CLI's shell output says `Exit Code: N` only when a command fails, so a
  command whose output has an `Error:` line of its own and no exit code is read as one
  that could not start, which is never a pass.
- Only Claude Code's transcripts are copied into the store. Codex keeps its sessions
  until you delete them; Gemini CLI deletes old ones when session retention is on, and
  then the report only sees what is left.
