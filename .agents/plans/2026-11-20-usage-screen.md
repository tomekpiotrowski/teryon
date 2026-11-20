# A usage screen for every agent's subscription

## 1. Overview

Teryon runs agents on subscriptions, so the question that matters is "how much of my limit is left, and when does it come back?", not "how much did it cost?". Each agent's company answers that differently, and some not at all outside their own program. This plan adds one screen, **Usage**, opened from the `/` menu or with `u`, that shows:

1. **Each agent's limits**: every limit window the program reports, as a bar with how much is used and when it resets.
2. **What Teryon's agents did**: tokens per day for the last 14 days, and by project and by model.

```
Usage
                                                          
Claude                                     as of 12 min ago
  5 hours        ██████░░░░░░░░░░░░░░  26%   resets in 2h 14m
  week           ███░░░░░░░░░░░░░░░░░  17%   resets Fri 18:30

Codex · Plus                                as of 3 h ago
  5 hours        ░░░░░░░░░░░░░░░░░░░░   0%   resets in 4h 10m
  week           ░░░░░░░░░░░░░░░░░░░░   2%   resets Thu 09:12

Grok             Grok shows its limit only in its own window

Last 14 days                              Claude  Codex  Grok
  ▁▂▅▃▁ ▇▆▂▃▅▁ ▂▃        tokens written per day, one agent at a time
  By project   sample-project   41 turns   2.1M read   38k written
  By model     Opus 5.5         30 turns   1.8M read   31k written
```

### What the programs offer (checked on 2026-11-20)

| | Limit windows | Plan name | Tokens per turn | Cost per turn |
|---|---|---|---|---|
| Claude 2.1.325 | yes, a `rate_limit_event` line in every turn (`unifiedWindows`: `five_hour`, `seven_day`, `seven_day_overage_included`, each with `utilization`, `resetsAt`, `status`) | no | yes | no |
| Codex 0.171.0 | not in its output, but in its session file `~/.codex/sessions/YYYY/MM/DD/rollout-…-<conversation>.jsonl`, `token_count` lines, `rate_limits.primary` / `secondary` (`used_percent`, `window_minutes`, `resets_at`) | yes, `plan_type` | yes | no |
| Grok 1.0.89 | no: `grok usage --json` answers "can't show your plan's usage here yet. Run /usage inside grok instead" | no | yes | yes, `total_cost_usd` |

### Decisions made here that we have not discussed

1. **One screen for the whole of Teryon, not per project.** Limits belong to an account, not a project. The project is one of the ways to split the 14 days.
2. **Limits are only as fresh as the last turn.** Teryon learns them when an agent runs, and doesn't call any company's servers on its own. Each agent's block says "as of 12 min ago". A window whose reset time has passed is shown as reset, at 0%.
3. **Codex's limits come from its own session files.** They are Codex's internal format, not a promise. If reading them fails, Codex shows "limits unknown" and the failure is logged as a warning.
4. **Grok shows no limits** and says why, until `grok usage --json` works outside its window. Teryon tries it once after each Grok turn, so it starts working on its own when Grok allows it. Its answer's format is unknown today, so this plan only stores it raw and shows nothing from it; reading it is a later change.
5. **Windows are a general list**, not fields per company: name, share used, reset time, reached or not. New windows a program starts reporting show up without changing the screen. Known names get labels ("5 hours", "week", "week, with extra usage"); unknown ones get their name with underscores turned into spaces.
6. **Teryon keeps its own log of turns** in the data folder (`usage.jsonl`, one line per turn) instead of reading every thread's conversation of every project when the screen opens. Turns before this change are not in it.
7. **Tokens are never added up across agents.** A Claude token and a Grok token are not the same amount of anything, so the chart shows one agent at a time, chosen with the buttons above it (and `1`, `2`, `3`).
8. **The chart shows tokens written; read appears on hover.** Read counts are dominated by text read again from the cache and would flatten the chart.
9. **Grok's dollar cost is not shown.** On a subscription it is not money anyone pays, and showing it next to agents that have no such number invites a false comparison.

## 2. Why?

Hitting a limit in the middle of work is the most common way an agent stops being useful, and today Teryon only says so after it happens, as a notice in the thread. With two or three subscriptions, the useful questions before starting are "which agent has room left?" and "will this one reset before I need it?". Each program answers only for itself and only in its own window. The 14 days answer a slower question: which projects and which models use the limits up.

## 3. Functional requirements

**Opening**
- The `/` menu has "Usage", key `u`, available with or without a project selected.
- `Escape` goes back to the screen shown before, like the other screens.

**Limits**
- One block per agent Teryon knows, in the menu's order.
- Header: the agent's name, and the plan name when known ("Codex · Plus"). On the right: "as of" and how long ago the reading was taken, updating every minute.
- One row per window: label, bar, share used as a whole percentage, and the reset time.
  - Reset within 24 hours: "resets in 2h 14m". Later: "resets Fri 18:30" (local time).
  - A window marked reached: bar and text in the error color, "reached, resets in …".
  - 80% or more: the warning color.
  - Reset time already passed: 0%, "reset since the last reading".
- No reading yet: "No limits seen yet. They appear after the agent's next turn."
- Grok: "Grok shows its limit only in its own window."
- Codex, file unreadable: "Codex's limits could not be read." and a warning in the log.
- After each turn the screen updates without reopening it.

**Last 14 days**
- Buttons for each agent that has turns in the log, first one chosen. Keys `1`, `2`, `3` choose.
- A bar per day for the chosen agent, tokens written, days with nothing shown as empty. Hover: the date, turns, read, written.
- Below: two short tables, by project and by model, for the chosen agent, largest first by tokens written: name, turns, read, written. Model names use the label rule from the model selection plan. Turns without a known model are "default".
- Numbers are shortened: 950, 12k, 2.1M.
- No turns in 14 days: "Nothing yet in the last 14 days."

**Recording**
- At the end of every turn, the author's and the reviewer's alike, one line goes to `usage.jsonl`: time, agent, model (when known), project, thread, tokens read and written.
- Claude: the last `rate_limit_event` of the turn becomes the agent's latest reading.
- Codex: after the turn, the last `token_count` line with `rate_limits` in the conversation's session file becomes the reading. The file is found by the conversation identifier at the end of its name, newest folders first. `CODEX_HOME` is respected.
- The latest reading per agent is kept in `limits.json` in the data folder, so the screen has something to show after a restart.
- A turn that ends with a "usage limit reached" notice marks the agent's shortest window as reached when the program gave no fresher reading.

## 4. Quality requirements

- **What failure costs:** low. A wrong number misleads, but nothing is lost and agents still run. The recording runs at the end of every turn, though, so it must never make a turn fail: every error there is logged and swallowed.
- **Unit tests** (complete over inputs):
  - Claude reading: one window, several, unknown window names, `status` reached, missing `unifiedWindows`, unreadable values, no event in the turn, several events (the last wins)
  - Codex reading: primary and secondary, only one, `plan_type` present and missing, no `rate_limits` line, a cut-off last line, file missing, several files matching, `CODEX_HOME` set and not
  - window labels: each known name, unknown names, window length given in minutes (Codex: 300 → "5 hours", 10080 → "week", others → "N hours" / "N days")
  - the usage log: writing a line, reading it back, skipping a broken line, leaving out lines older than 14 days
  - in the application: reset text (passed, within an hour, within a day, later), warning and error levels at the edges (79%, 80%, 100%, reached), number shortening, days with no turns, grouping by project and by model, the agent buttons with one, two and no agents
- **Request tests:** asking for usage with no log and no readings; after a Claude turn; after a Codex turn with a fake session file; the event sent after a turn carries the new reading.
- **End-to-end:** one test. Run a turn with the fake Claude program, open Usage with `u`, see the 5 hours window and one day's bar.
- **Monitoring:** a warning in the log when a reading or the log line fails. Nothing more.
- **Conventions:** `lets_expect!`, `thiserror`, imported types, inline documentation. Observables in the application, NgModules, zoneless, bun. The chart follows the `dataviz` skill.

## 5. Risks

1. **Codex's session files can change form with any Codex release.** The reading fails softly and says so. The sample file in the tests makes a change visible when the samples are recorded again.
2. **`seven_day_overage_included` is a guess at a label.** Its meaning isn't documented. If it confuses, it can be left out by name.
3. **Limits go stale between turns.** "as of" says so. A fresher source would mean calling each company's servers with the person's sign-in, which Teryon doesn't do.
4. **A program used outside Teryon uses the same limits.** The limits are right (they are the account's), but the 14 days only show Teryon's turns. The section title says "Teryon's agents" to be clear about that.
5. **Grok gets the least.** Its block says why, and the check after each turn picks up `grok usage --json` once it works.

## 6. Out of scope

- Reading `grok usage --json` once it works (its format is unknown today)
- A curve of each window over time, from a history of readings
- Warnings outside this screen, such as on the reply box when an agent is above 80%
- Choosing another agent automatically when one is out of room
- Money: cost estimates, API-key accounts
- Turns from before this change, and from programs run outside Teryon
- Sharing usage between people on a shared server

## 7. Complexity

Medium. One new reading per driver, one small log, one request, one event and one screen. The model selection plan should go first, so turns carry their model; without it the "by model" table only has "default".

## 8. Implementation details

### Service

- `core/src/usage/mod.rs` (new):
  - `Window { name, label, used: f64, resets_at: Option<DateTime<Utc>>, reached: bool }`
  - `Limits { agent, plan: Option<String>, windows: Vec<Window>, taken_at }`
  - `UsageLine { at, agent, model, project_id, project_name, thread_id, read, written }`
  - `UsageError` with `thiserror`.
- `core/src/usage/limits.rs`: reading Claude's event, reading Codex's session file, window labels, keeping `limits.json` (written with `store::replace_file`).
- `core/src/usage/log.rs`: appending to and reading `usage.jsonl`, keeping the last 14 days when read.
- `core/src/agents/mod.rs`: the driver's parsed output gets a limits reading next to its entries, so a driver can report one without it becoming a thread entry. `AgentDriver` gets `fn limits_after(&self, turn: &Turn) -> Option<Limits>`, empty by default; Codex implements it with the session file.
- `core/src/agents/claude.rs`: `rate_limit_event` becomes a reading instead of being skipped.
- `core/src/agents/runner.rs`: at the end of a turn, write the log line and the reading, log and swallow errors, and send the event.
- `core/src/protocol.rs`: request `Usage` answered with `UsageReport { limits: Vec<Limits>, lines: Vec<UsageLine> }`; event `LimitsChanged { limits }`; event `UsageRecorded { line }`. Regenerate the TypeScript.
- `service/src/session.rs`: route the request.

### Application (`app/src/app/workspace/`)

- `usage.model.ts`: everything the screen shows as plain functions: window rows, reset text, levels, day bars, tables, number shortening. Takes the current time as an input so it can be tested.
- `usage.component.ts` / `.html`, declared in `workspace.module.ts`. The view is one observable built from the report, the events and a one-minute timer.
- `commands.model.ts`: the "Usage" command, key `u`; `showScreen` gains `'usage'`.
- `workspace.api.ts`: the request and the events.
- `e2e/usage.spec.ts`: the one test. The fake Claude program prints a `rate_limit_event`.

### Steps

Each step: write the failing tests, make them pass, commit.

1. Types, window labels, `limits.json`, the usage log.
2. Claude's reading from its event.
3. Codex's reading from its session file, with a sample file in `core/tests/samples/agents/`.
4. Recording at the end of a turn; the request and the events; request tests.
5. `usage.model.ts`.
6. The screen and the command.
7. The end-to-end test. Then run Teryon, do one turn with Claude and one with Codex, and compare the screen with `/usage` in each program.

## 9. Notes from implementing

Where the result differs from the plan above:

- **Leaving the screen** is with `u` again or the "usage" link at the bottom, like the files (`c`) and the history (`h`). Neither of those closes on Escape, so the usage screen doesn't either.
- **Each turn records the model its summary names**, from the model selection change, and the "by model" table shows its label ("Opus 5.5"). A turn whose model is unknown, such as Codex on its default, shows as "default".
- **Text uses whole words** ("as of 12 minutes ago"), except reset times, which follow the thread's existing "2h 14m" style.
- **The end-to-end test checks the "reset since the last reading" case**, because the recorded Claude turn's reset times (14 November 2026) have passed. The other cases are covered by the unit tests.
- **`[agents.codex] home`** was added to `config.toml`, so tests never read a person's real `~/.codex`.

For later:

- `usage.jsonl` only grows. Reading it skips lines older than 14 days, but nothing removes them. At one line per turn this takes years to matter. Trimming it when it is read would fix that.
- Events that arrive between asking for the report and getting the answer are missed until the screen is opened again.
