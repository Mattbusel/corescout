# Connecting an AI

CoreScout is a Model Context Protocol server. Any client that speaks MCP can
use it, and the generic path is the real path: the per-client instructions
below are conveniences that know where each one keeps its configuration file.

---

## The one line

```bash
# Claude Code
claude mcp add corescout --scope user -- "C:\Program Files\CoreScout\corescout-mcp.exe"
```

```jsonc
// Cursor: %USERPROFILE%\.cursor\mcp.json
{ "mcpServers": { "corescout": { "command": "C:\\Program Files\\CoreScout\\corescout-mcp.exe", "args": [] } } }
```

```toml
# Codex: %USERPROFILE%\.codex\config.toml
[mcp_servers.corescout]
command = "C:\\Program Files\\CoreScout\\corescout-mcp.exe"
args = []
```

```jsonc
// OpenCode: %USERPROFILE%\.config\opencode\opencode.json
{ "mcp": { "corescout": { "type": "local", "command": ["C:\\Program Files\\CoreScout\\corescout-mcp.exe"], "enabled": true } } }
```

The AI screen in the application generates all of these with the real path
filled in, and offers to write the file where the format is one CoreScout can
safely edit. It parses the existing file first and refuses rather than
rewriting one it could not read, because a configuration file rebuilt from a
failed parse is somebody's editor setup destroyed.

Claude Code is configured through its own command rather than by editing its
file, because that file's format belongs to Claude Code and not to CoreScout.

---

## What the AI is told when it connects

The initialise handshake carries an `instructions` field, which clients put in
front of the model. CoreScout generates it from live state, so it cannot
advertise a mode that is not in force or a capability that is switched off:

```text
You are operating inside a computer running CoreScout.

CoreScout keeps an empirical model of this machine and of how AI tools have
worked with it. You can query it for operational knowledge that outlives your
session.

Right now it distinguishes 32 recurring states of this machine, holds 4
operational patterns, and has 2 verified procedures (1 approved for use).

CoreScout separates association from causal evidence. Anything labelled "Seen
together" is a correlation it has not tested; anything labelled "Verified" was
measured under randomised assignment. Check which before you rely on it.

Useful things to ask it:
  - what it has learned about this repository
  - whether an operation has a recurring failure mode here
  - whether there is a verified procedure for what you are about to do
  - what state the machine is currently in, and whether it is unusual

Report what you do back to it. Tool calls, commands, exit codes, and above all
whether you actually checked the result. CoreScout learns from the difference
between what a tool reported and what turned out to be true.

CoreScout is in Suggest mode: it can propose changes, and the user approves
each one. You may ask it to propose.

Do not treat CoreScout's claims as certain. Every answer carries its evidence
and its confidence; read them.
```

Nobody has to write CoreScout into a prompt, and the description cannot drift
out of date, because it is produced at connection time.

---

## The tools

Fourteen. Twelve read, two do something.

| tool | what it answers |
|---|---|
| `corescout_status` | Is CoreScout running, what is connected, how much has it seen |
| `corescout_briefing` | What it is, what it knows, what it will and will not do now |
| `corescout_knowledge` | Everything learned, grouped: about AI tools, repositories, the machine, and what is still uncertain |
| `corescout_current_state` | What the machine is doing, and whether this is a state it recognises |
| `corescout_machine` | What this computer physically is |
| `corescout_learned` | Every learned item, each labelled with how it is known |
| `corescout_failures` | Operations that recur and go wrong here |
| `corescout_capabilities` | Verified procedures the user has approved |
| `corescout_hypotheses` | What is being tested, and what evidence is missing |
| `corescout_ask` | A question about a repository, an operation, or the machine |
| `corescout_explain` | The evidence behind one belief |
| `corescout_recent_activity` | What has happened recently |
| `corescout_observe` | **Report what you just did and how it went** |
| `corescout_run_capability` | Run a verified procedure, subject to permissions |

Every tool maps to a method in the product API, and a test asserts that
mapping, so a tool cannot exist that the application itself cannot perform.
There is no second code path with different rules: the same permission check
runs whether a request came from the window, the command line, or an agent.

### `corescout_observe` is the one that matters

Everything CoreScout learns about AI work comes through it, and the field that
carries the most information is `verified`:

```jsonc
{
  "session": "a stable id, the same one each time",
  "name": "cargo build --release",
  "kind": "build",
  "exit_code": 0,
  "reported": "success",          // what the tool said. Omit if it said nothing.
  "verified": "contradicted",     // what you checked. Omit if you did not check.
  "verification_detail": "the binary was not produced"
}
```

`reported` and `verified` are separate fields because they disagree, and every
disagreement is the useful part:

```text
reported success + verified contradicted  →  a silent failure
reported success + nothing verified       →  an unknown
reported success + verified confirmed     →  it worked
```

A schema that collapsed these into `success: true` could not represent "the
deploy said it worked and the service was not reachable", which is the first
thing this product exists to notice. So: do not guess. Leaving `verified` out
is honest; filling it in from an exit code is not, and it teaches CoreScout
that a broken procedure is reliable.

Secrets are stripped from `name` and every other string before anything is
written down.

---

## Questions worth asking

```text
What have you learned about this repository?
Are there recurring failure modes for this operation?
Is there a verified way to do this?
What happened the last time an agent tried this?
Do you have evidence that this procedure is better?
What state is the machine in, and is anything unusual about it?
```

`corescout_ask` routes these by keyword. That is all it is, and it says so: the
answer always carries the structured evidence beside the sentence, and a
question it cannot route is told so rather than answered from the nearest
thing.

---

## If CoreScout is not running

The bridge starts the service if it is not there, waits about ten seconds, and
carries on either way. If it cannot, the handshake still completes and the
tools still list; each call returns:

> CoreScout is installed but its service is not running, so it has nothing to
> tell you right now. Opening the CoreScout app will start it. Nothing else is
> affected.

That is a tool result rather than a transport error, deliberately. A model that
gets a broken pipe learns nothing; a model that gets that sentence can say it
to the user.

The bridge starts the service detached, because MCP clients kill their servers
when a session ends. Without that, CoreScout would be started by the first
agent session and stopped by it, and a user would find it had forgotten
everything between conversations.

---

## Writing another integration

The generic path is the whole interface. If a client speaks MCP, point it at
`corescout-mcp.exe` over stdio and there is nothing else to do.

If a client does not speak MCP, the local API is a loopback JSON endpoint:

```http
POST /call
Authorization: Bearer <token from %LOCALAPPDATA%\CoreScout\endpoint.json>
Content-Type: application/json

{"method": "ask", "params": {"question": "what have you learned here?"}}
```

`GET /methods` lists everything. `GET /health` needs no token. The port is
whatever the service was given, and is in the same endpoint file.
