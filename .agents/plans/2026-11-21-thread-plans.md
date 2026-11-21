# A thread's plan, kept where Teryon knows

## 1. Overview

Testing stages showed a thread jump from Planning to Implementing while the agent was only writing its plan. The agent made a worktree to write the plan in, and Teryon reads "a change started" as the start of implementing. That skipped the person's approval, the one thing that should take a thread out of Planning. A plan written somewhere in the project, or in a worktree, is also hard to find and to talk about.

This plan:

1. **Gives every thread one plan file**, `.teryon/threads/<thread>/plan.md`, next to its conversation. The agent is told the path and writes there, never in the project or a worktree.
2. **Shows the plan in the thread.** Each time a turn writes or changes it, a line appears: `◇ Plan written · Thread plans`, later `◇ Plan changed · Thread plans`. Clicking it opens the plan.
3. **Adds a Plan screen**, on `p`, showing the current plan as rendered markdown, like History and Files.
4. **Stops a worktree from moving a thread out of Talking or Planning**, on workflows that have Planning. Only the agent reporting `implementing`, after the person approved, does that.

```
❯ Let's add plans to threads
◇ Talking → Planning · a plan was written
◇ Plan written · A thread's plan
  1. Where should plans be kept? …          ← the agent's questions
❯ Answers to your questions: …
◇ Plan changed · A thread's plan
❯ Looks good, go ahead
◇ Planning → Implementing · by Claude
```

### Decisions you made

1. **The plan lives in `.teryon/threads/<thread>/plan.md`.** It goes with the thread and never into the project's history.
2. **It shows as a line in the thread whenever it is written or changed, and on a Plan screen with a key.**
3. **Starting a worktree never moves the stage out of Talking or Planning** on workflows with Planning. On workflows without it (`small`), Talking → Implementing on a worktree stays, since there is nothing to approve there.

### Decisions made here that we have not discussed

1. **The agent writes the file itself; Teryon looks after each turn.** No new command: a plan is long and rewritten bit by bit, which agents do well with their own file tools. Teryon compares the file after every turn of the thread's agent, and when the person opens the thread, so an edit by hand shows too.
2. **Only the current plan is kept** (your choice when approving). Teryon remembers a fingerprint of the file to tell when it changed; every line opens the plan as it is now.
3. **A plan written while Talking moves the thread to Planning**, on workflows with Planning. Teryon sees it happen, like a change starting, so the agent needn't report it.
4. **Every agent of the thread is told where the plan is**, in every stage, once a plan exists:
   - the author: "The plan for this thread is in `<path>`. Keep it current when the plan changes."
   - in Talking and Planning: "Write the plan as markdown in `<path>`, not in the project and not in a worktree. Don't create a worktree before the person approves it."
   - the reviewer: "The change was built from the plan in `<path>`; check it against the plan."
   - the message that starts Implementing by itself becomes "The plan in `<path>` is approved. Implement it." This also fixes the weakest part of handing a thread to another program: the new agent reads the plan itself instead of a summary of the conversation.
5. **The line names the plan by its first heading**, or "Plan" without one.
6. **A plan larger than 1 MB is not copied or shown**; the line says it is too large to show.
7. **The plan stays after the change is applied.** Copying it into the change, as `.agents/plans/` is used today, is left for later.

## 2. Why?

Approval of a plan is the point where the person decides what gets built, and with automatic starts, the point where another agent may take over. It has to be reliable, and it has to be about something the person can read. Today the plan is a message somewhere in a long thread, or a file wherever the agent chose to put it. One known file per thread, shown in the thread and on its own screen, makes it something to read, discuss and approve, and something the next agent can be pointed at.

## 3. Functional requirements

- The brief of the thread's agent includes the plan's path as in decision 4, in every turn. The reviewer's brief includes it once a plan exists.
- After a turn of the thread's agent ends, and when the person follows the thread, Teryon reads `plan.md`:
  - missing or unchanged: nothing happens
  - new or changed: set `thread.plan = { title, fingerprint }`, add a `plan` entry `{ title, first: bool }`
  - on the first plan while in Talking, on a workflow with Planning: move to Planning, "a plan was written"
  - larger than 1 MB: a `plan` entry saying it is too large to show
- `Happening::ChangeStarted` moves the stage only on workflows without Planning.
- New request `Plan { thread_id }` answers with the plan's text; protocol and TypeScript regenerated.
- App:
  - a `plan` entry is a line: "◇ Plan written · <title>" or "◇ Plan changed · <title>", clickable, opening the Plan screen
  - the Plan screen: bar "plan · <thread title>", rendered markdown, Esc back; it reads the plan again when it changes
  - menu command "Plan" on `p` while the open thread has one
  - the board's card shows "plan" when the thread has one

## 4. Quality requirements

- **What failure costs:** high. A thread leaving Planning without approval can start another agent building the wrong thing.
- **Unit tests:** the stage rule for a worktree × every stage × workflows with and without Planning; the brief in every stage with and without a plan, author and reviewer; reading the first heading (none, several, not on the first line); detecting a change (missing, same, changed, emptied, too large); the app's plan line, the screen, the menu command.
- **Request tests:** a turn writing the plan adds a "Plan written" line and moves Talking to Planning; a second turn changing it adds "Plan changed"; an unchanged plan adds nothing; a worktree during Planning leaves the stage; on `small` it still moves to Implementing; a plan edited by hand shows when the thread is followed; `Plan` answers the text; the automatic start of Implementing names the plan's path.
- **End-to-end:** one test: the fake agent writes a plan, the line shows, `p` opens it rendered, and a change shows on the open screen.
- **Monitoring:** none beyond the line in the thread; a plan that can't be read is logged as a warning.
- **Conventions:** `lets_expect!`, `thiserror`, imported types, inline documentation; observables, NgModules, zoneless, bun.

## 5. Risks

1. **Agents may still write the plan elsewhere**, or make a worktree first. The brief says where, and a worktree no longer moves the stage, so the cost is a plan Teryon doesn't show, not a skipped approval.
2. **Grok's sandbox must allow writing `.teryon/threads/`.** It is inside the project, which an author may write; checked in the first step.
3. **A plan the agent rewrites many times in one turn** shows as one change, since Teryon looks only when the turn ends. That is the useful granularity anyway.

## 6. Out of scope

- Keeping earlier versions, and the difference between them
- Copying the plan into the change when it is applied
- Editing the plan in Teryon
- Plans for the reviewer to write

## 7. Complexity

Small to medium: one file watched at the end of a turn, one entry kind, one request, one screen built like History.

## 8. Implementation details

- `core/src/plans.rs` (new): the path, reading and comparing (`Plan::read`), the title. `PlanInfo { title, fingerprint }` on `Thread`, `#[serde(default)]`.
- `core/src/agents/staging.rs`: the plan lines of the brief; `changes/reviewing.rs`: the reviewer's line.
- `core/src/workflow/mod.rs`: `ChangeStarted` only without Planning; new `Happening::PlanWritten`.
- `core/src/workspace/`: look at the plan in the `ThreadChange::Finished` arm before `carry_on`, and in `refresh_change`; `plan(thread_id)`; the opening message names the path.
- `core/src/protocol.rs`, `service/src/session.rs`: the request.
- App: `plan.component.ts` and the screen's model, the `plan` line in `workspace.model.ts`, the screen in `workspace.page`, the command in `commands.model.ts`, the card in `board.model.ts`.
- `core/tests/samples/fake-agent.sh`: "write a plan <title>" writes `plan.md` at the path from its brief.

### Steps

Each step: failing tests, then the code, then commit.

1. Check Grok writing `.teryon/threads/` from its sandbox.
2. The worktree rule and `PlanWritten`.
3. Reading and comparing; the entry and thread field; request tests.
4. The briefs and the opening message.
5. The `Plan` request.
6. The app: line, screen, command, card.
7. The end-to-end test; then run Teryon and plan something real with Claude, Codex and Grok.
