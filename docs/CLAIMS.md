# Did the AI check before it said "done"?

Two commands answer that for Claude Code, from the transcripts Claude Code already
writes to `~/.claude/projects/**/*.jsonl`. Neither needs the CoreScout service
running, and neither sends anything anywhere.

- `corescout report` looks back over your history.
- `corescout gate` is a Claude Code `Stop` hook that stops it from ending a turn on an
  unchecked "done".

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

A **check** is a `Bash` or `PowerShell` tool call that runs tests, a build or a
linter, for example `cargo test`, `cargo clippy`, `npm test`, `npm run build`,
`pnpm lint`, `npx tsc`, `pytest`, `python -m unittest`, `go test`, `make`,
`./gradlew test`, `dotnet build`, `ruff check`, `eslint`. `cd app && npm test` counts;
`echo cargo test` does not. A check **passed** when Claude Code recorded exit code 0
and the output carries no unambiguous failure marker (`test result: FAILED`,
`error: could not compile`, `npm ERR!`, ...). That second rule exists because
`cargo test | tail` exits with `tail`'s code. A call that a hook refused, that was
interrupted or that was sent to the background has no exit code and is not a pass.

## `corescout report`

```
corescout report                 # the last 30 days
corescout report --days 7        # the last week
corescout report --top 25        # list 25 recent unbacked claims instead of 10
corescout report --card          # a one-screen card you can paste anywhere
corescout report --json          # everything, for a program
corescout report --no-store      # read in place, copy nothing
corescout report --from <DIR>    # only the transcripts in DIR, read in place
```

It prints the share of success claims in the window that had **no passing check in
that turn or in the turn just before it**, then the most recent unbacked claims (ten,
or `--top <N>`), each with its time (UTC), session, project folder, the sentence that
made the claim, and the last command that ran before it with its exit code.

`--card` keeps only counts and the kinds of claim: no paths, session ids, project names
or message text. `--json` returns `claims`, `unbacked`, `unbacked_percent`,
`sessions`, `projects`, `by_phrase` and `top_unbacked` (each with `session`,
`project`, `timestamp`, `claim`, `phrase`, `last_command`, `last_exit`).

`--from <DIR>` reads only the transcripts under `DIR`, where they are. It never copies
them into the store and never reads the store, so a report on another folder is a
report on that folder alone and cannot change the history kept for
`~/.claude/projects`.

A `--days` value larger than your history simply means "everything".

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

When Claude Code is about to end a turn, the gate reads the transcript. If the final
assistant message claims success and **no check passed in that turn**, it exits 2 and
writes a message to stderr. Claude Code shows the message to the model and keeps the
turn going, so it runs the check or says plainly that the change is unverified.

It lets the stop through (exit 0) when:

- the final message claims nothing,
- a check passed in the turn,
- the stop is already the result of a block (`stop_hook_active` is true), so it can
  never loop, or
- it cannot read the hook payload or the transcript. A gate that breaks your session
  because of its own bug is a gate you would uninstall.

### Setting it up

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
- Only `Bash` and `PowerShell` tool calls are read as checks. A test run through some
  other tool (an MCP server, a background task read later) is not seen.
