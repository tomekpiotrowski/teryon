# Thread stages, an agent for each stage, and a board

## 1. Overview

Today a thread is either running or waiting, and nothing says where its work stands. This plan:

1. **Gives every thread a stage**: Talking, Planning, Implementing, In review, Ready to apply or Done.
2. **Lets workflows choose stages.** A workflow is an ordered pick from the fixed stages, defined in `config.toml`. A documentation thread can skip planning and review. The agent chooses the thread's workflow, and you can change it.
3. **Moves the stage** from what Teryon sees (a change started, a review ended, a change applied), and from what the agent reports with `teryon thread stage <stage>`. You never set a stage directly: you steer the work, by messages and by actions such as asking for a review or applying a change, and the stage follows the work.
4. **Gives each stage its own provider and strength**, set in `config.toml`. Each provider (Claude, Codex, Grok) names three of its models as *strong*, *medium* and *fast*. A stage asks for a strength, and for a provider in one of three ways: *any*, picked at random among those with usage left; *prefer* one, falling back to another when its usage runs out; or *only* one, with an error when its usage runs out. When the stage changes and that stage wants a different agent or model, Teryon starts it right away, without waiting for a message from you.
5. **Shows a board**: one column per stage and one card per thread, for the current project, or for every project with a separate command.

What a thread goes through on the default workflow, with the settings in section 3:

```
Talking        Claude · Opus       prefer Claude, strong · you describe what you want
Planning       Claude · Opus       prefer Claude, strong · writes a plan, asks you to approve it
Implementing   Codex · astra       only Codex, medium · starts by itself once the plan is approved
In review      Grok · 4.7          any, strong · picked at random among Claude and Grok
  findings: 3  → Implementing      Codex gets the findings by itself
In review      Grok · 4.7          starts by itself again, same reviewer
  findings: 0  → Ready to apply    waits for you
Done                               you applied the change
```

The board:

```
 Talking (1)        Planning (1)       Implementing (2)    In review (1)     Ready to apply (1)
 ┌──────────────┐   ┌──────────────┐   ┌───────────────┐   ┌──────────────┐  ┌──────────────┐
 │Usage screen  │   │Thread stages │   │Fix login      │   │Code explorer │  │Model labels  │
 │Claude · Opus │   │Claude · Opus │   │Codex · astra ●│   │Grok ●        │  │4 files       │
 │? 2 questions │   │              │   │fix-login · 3  │   │              │  │0 findings    │
 └──────────────┘   └──────────────┘   └───────────────┘   └──────────────┘  └──────────────┘
                                       ┌───────────────┐
                                       │Docs: install  │
                                       │Claude · Haiku │
                                       └───────────────┘
 h j k l move · Enter open · H L move to another stage · Esc back
```

`●` means an agent is running, `?` that it waits for your answers.

### Decisions you made

1. **The stages are a fixed set, and workflows pick from them.** Teryon knows what each stage means and acts on it. Workflows in `config.toml` choose which stages a thread goes through.
2. **When the stage changes, the stage's agent starts right away**, without waiting for a message from you.
3. **The agent chooses the workflow, and you can change it.** A thread starts on the default workflow.
4. **The board shows the current project.** A second command shows every project.
5. **Each provider names a strong, a medium and a fast model.** A stage asks for one of the three strengths, not for a model by name.
6. **A stage chooses its provider in one of three ways:** *any* (at random), *prefer* one provider (another when its usage runs out), or *only* one provider (an error when its usage runs out).

### Decisions made here that we have not discussed

1. **Stage and activity are separate.** Running, waiting for your answers and idle stay as they are today (`Status`, `asking`, `unread`). A card in "In review" can be running or not. The board shows activity as a mark on the card, not as a column.
2. **The six stages.** `talking`, `planning`, `implementing`, `review`, `ready`, `done`. Only `talking`, `planning` and `implementing` are optional in a workflow. `ready` and `done` are always there, because a change has to end somewhere. `review` can be left out. A workflow needs at least one of `talking`, `planning` or `implementing` to start in.
3. **Two built-in workflows**, which `[workflows]` in `config.toml` replaces as a whole, as the model lists do:
   - `default`: talking, planning, implementing, review, ready, done
   - `small`: talking, implementing, ready, done
4. **An agent is set per stage, not per move between two stages.** "Who works in this stage" is simpler to write and covers every case we discussed. Moving back to Implementing after a review uses Implementing's agent, as moving there from Planning does.
5. **The three ways are written `any`, `prefer` and `only`.** In `config.toml`: `provider = "any"`, `provider = "prefer claude"`, `provider = "only codex"`. On screen: "Any provider", "Prefer Claude", "Only Codex". *Only* is the name for the mode you were looking for: short, and it reads naturally in both places.
6. **Built-in strengths:** Claude's are `opus`, `sonnet` and `haiku`, short names that always mean the newest version. Codex and Grok have no built-in strengths, since any list written into Teryon would go stale. `[agents.codex] strong = "…"` and so on set them. **A strength a provider doesn't set runs the program's default model**, and the stage line says so ("Codex has no fast model set, so it runs its default"). The alternative, leaving such a provider out, would make Codex and Grok unusable until configured.
7. **"Usage left" comes from the limits the usage screen already reads.** A provider has run out when one of its windows is `reached` and resets in the future, or has no reset time. A provider never read, or whose reading failed, counts as having usage left: Teryon doesn't know better, and the turn will say if it's wrong. Limits are only as fresh as that provider's last turn.
8. **Which providers *any* picks from:** those whose program is found on this machine and that have usage left. A stage can narrow this with `among = ["claude", "grok"]`. With nothing left to pick from, it's the same error as *only*.
9. **A pick sticks to the thread for that stage.** When the thread comes back to Implementing after a review, it gets the same provider as before, so a loop of fixes doesn't hand over between programs every round. The pick is made again only when that provider has run out, or when you change the thread's workflow or agent. *Prefer* is the exception: it goes back to the preferred provider at the next start of the stage once its usage is back.
10. **Review never picks the author's provider**, since Teryon refuses an agent reviewing its own change. *Any* and *prefer* skip it as they skip a provider that ran out. *Only* naming the author's provider is an error. `config.toml` is refused when a workflow has both Implementing and Review set to *only* the same provider, since that can never work.
11. **When a provider runs out in the middle of a stage**, the turn ends with the program's limit message, as today. On *any* and *prefer*, Teryon picks another provider and starts it on the same stage, with the thread so far through the handover; it counts as an automatic round. On *only*, the thread waits with the error.
12. **The error on *only*** is a line in the thread, "Codex's usage is used up until 14:00, and Implementing allows only Codex", the thread is marked unread, and its card shows the same in short. Starting it again by itself once the limit resets is left for later.
13. **A stage without a setting keeps whatever agent and model the thread has.** Without any `[stages]` in `config.toml`, Teryon behaves as it does today, apart from showing stages.
14. **`ready` and `done` have no agent.** They wait for you: Apply, Send for review on GitHub, Discard.
15. **Teryon starts the next agent only when the turn that changed the stage has ended.** Stopping an agent in the middle of a turn would cut off its last message. If the new stage wants the same agent and model, nothing new starts: the agent simply carries on in its own turn.
16. **What the next agent is told:**
   - Implementing, after Planning: "The plan above is approved. Implement it." with the thread so far, through the handover that already exists when the program changes.
   - Implementing, after a review with findings: the reviewer's last message, as if you had passed it on.
   - In review: the review message used today ("Review the change." or "…review it again…").
   - Talking and Planning are never entered automatically, apart from the start of a thread, so they need no message.
17. **Findings go back to the author by themselves.** Today you decide what the reviewer's findings lead to. With automatic hand-over, a review with findings moves the thread back to Implementing and sends the findings to the author. A review with `Findings: 0` moves the thread to Ready to apply. A review that ended without a count, or was cut short, stays in In review and waits for you.
18. **A limit on rounds without you.** After 3 automatic starts in a row without a message from you, Teryon stops starting agents. The thread waits in its stage with a note: "Stopped after 3 rounds without you. Send a message to carry on." Your message resets the count. This stops an author and a reviewer who never agree from using up a usage limit. `config.toml` can change the number: `[workflow] automatic_rounds = 3`, where 0 turns automatic starts off.
19. **The agent must not leave Planning by itself.** Its brief tells it to ask you to approve the plan with `teryon thread ask` and move to Implementing only after you agree. Teryon doesn't enforce this, because it can't tell approval from other replies, but it does refuse the agent moving from Planning to anything other than Implementing or Talking.
20. **What Teryon moves by itself:**
    - a new thread starts in the workflow's first stage
    - `teryon worktree create` moves a thread in Talking or Planning to Implementing
    - asking for a review, from the menu, moves it to In review
    - a review's verdict, as in decision 17
    - applying, discarding or the change being accepted on GitHub moves it to Done
    - a message from you to a thread in Done moves it to the workflow's first stage, since its next message can start a new change
21. **What the agent may report** with `teryon thread stage <stage>`: any stage in its workflow except `done`, which only finishing a change reaches. Reporting a stage the workflow skips is refused with the list of stages it has. The reviewer can't report stages; its verdict does that.
22. **Stages move only as a result of work.** There is no command to set a stage and the board is for looking, not for moving cards. A stage describes where the work stands, so it changes when the work does. When a thread is stuck in the wrong stage, you tell the agent, for example "The plan is fine, go ahead", and it reports the stage itself. The cases where nobody would report one:
    - an agent forgot to report: your message reminds it, and its brief says to report the stage when the conversation has moved on
    - a review ended without a count and waits in In review: you write to the author, whose brief in In review says to report `implementing` if it goes back to work on the change, or to the reviewer, whose next review gives a count
    - an *only* provider ran out: your next message tries again
    - the round limit stopped a thread: your next message carries on
23. **The agent picks the workflow with `teryon thread workflow <name>`.** Its brief lists the workflows with one line about each, from a `description` in the settings. It can pick only while the thread is still in its first stage, and while no change has started, so that a thread doesn't jump workflows halfway. You can change it at any time; a thread in a stage the new workflow skips moves to the next stage the new workflow has.
24. **Every stage change is a line in the thread**: "Planning → Implementing · by Claude", "In review → Ready to apply · 0 findings", "Implementing → In review · you asked for a review". You can see later why a thread is where it is.
25. **Stage names can be relabelled**, for everyone, not per workflow: `[stages.ready] label = "Ready to merge"`. The internal names stay fixed, so the board's columns and, later, GitHub's statuses line up by those names.
26. **The board shows only the stages the visible threads' workflows use**, in the fixed order. Done shows the 10 most recently finished threads, folded by default.
27. **The new-thread button starts on the first stage's choice.** Choosing another agent there is for that thread only, as today. Changing a thread's agent from the menu lasts until the thread's next stage change.

### Changed while implementing

1. **Automatic starts begin only once `config.toml` sets `provider` or `strength` for some stage.** Without that, findings, reviews and the next stage wait for you as before. This is how decision 13 holds while decision 17 applies.
2. **The board of every project has no key of its own.** Keys don't tell `b` from `B`, so `b` opens the board of the open project, `a` on the board switches to every project and back, and "Board of every project" is in the menu.
3. **Cards say "2 rounds without you"**, not "round 2 of 3": the limit isn't sent to the app.
4. **Done shows its 10 newest threads, with "and N older"** below them, instead of being folded.
5. **A thread's agent changed from the menu is remembered as the pick for its stage**, so "any" and "prefer" keep the person's choice for the rest of that stage.
6. **New threads start on the first stage's choice until you pick an agent.** The button says so ("Talking: Prefer Claude · strong") and cycles through that choice, Claude, Codex and Grok. A thread starting over after Done also gets its first stage's agent.
7. **Choosing a workflow while nobody works starts the new stage's agent**, as a turn ending there would, with "Implement what was discussed above." rather than saying a plan was approved.
8. **The board orders each column by when threads moved there**, so Done keeps what was finished most recently.

## 2. Why?

Teryon now runs several threads at once, each with an agent, sometimes a reviewer, a change and questions. The thread list says which ones are running, but not which need a plan approved, which are being reviewed and which only wait to be applied. A stage on every thread, and a board that shows them side by side, answers "what should I look at next?" in one screen.

Different work suits different models, and the previous plan made the model a choice. Choosing it by hand at every step is the kind of work Teryon should take off you. Tying the choice to the stage, and starting the next agent by itself, turns a thread into a pipeline: you talk, approve the plan, and come back to a reviewed change.

Fixed stages keep that possible. Teryon can only start a review, or offer Apply, if it knows what a stage means. Free-form stages would be names with nothing behind them, and threads on different workflows couldn't share a board. Later, when projects live on a Teryon server or in GitHub Projects, the fixed names map onto their status fields.

## 3. Functional requirements

**Settings**

```toml
[workflow]
default = "default"          # the workflow new threads start on
automatic_rounds = 3         # automatic starts in a row without you; 0 turns them off

[workflows.default]
description = "Anything that needs thinking through: features, bugs, changes to design"
stages = ["talking", "planning", "implementing", "review", "ready", "done"]

[workflows.small]
description = "Small, clear changes: documentation, wording, settings"
stages = ["talking", "implementing", "ready", "done"]

# Each provider's three strengths. Claude's are built in; these replace them.
[agents.claude]
strong = "opus"
medium = "sonnet"
fast = "haiku"

[agents.codex]
strong = "gpt-6-astra"
medium = "gpt-6-astra-mini"
fast = "gpt-6-astra-nano"

[agents.grok]
strong = "grok-4.7"
fast = "grok-4.7-build-fast"        # no medium: medium runs Grok's default model

[stages.talking]
provider = "prefer claude"
strength = "strong"

[stages.planning]
provider = "prefer claude"
strength = "strong"

[stages.implementing]
provider = "only codex"
strength = "medium"

[stages.review]
provider = "any"
among = ["claude", "grok"]          # optional; leaving it out means every installed provider
strength = "strong"

[stages.ready]
label = "Ready to merge"
```

- `[workflows]` replaces the built-in workflows when present.
- A workflow's `stages` must keep the fixed order, include `ready` and `done`, and start with one of `talking`, `planning` or `implementing`. A workflow breaking this, an unknown stage, or `workflow.default` naming a missing workflow, stops the service from starting with an error naming the line, as other invalid settings do.
- `provider` is `"any"`, `"prefer <agent>"` or `"only <agent>"`; `strength` is `"strong"`, `"medium"` or `"fast"`. A stage with one of them takes the other's default: `"any"` and `"medium"`. Anything else, an unknown agent in `provider` or `among`, `among` beside `only`, `provider`, `strength` or `among` on `ready` or `done`, or Implementing and Review both `only` the same agent in one workflow, is invalid.
- Models named as strengths aren't checked, as in the model plan: an unknown one gives the program's own error in the thread.
- Strengths join the model lists from the model plan: each strength's model is offered in the menu even when the `models` list leaves it out, and the menu shows the strength beside it, "Claude · Opus (strong)".

**Stages on a thread**

- A new thread gets the default workflow and its first stage.
- Stage changes follow decisions 20 to 23.
- Every change adds a `stage` entry to the thread: from, to, and who moved it (you, the agent's label, or Teryon with its reason).
- `teryon thread stage <stage>` and `teryon thread workflow <name>` work like `teryon thread title`: only during the thread's agent's turn, refused for the reviewer, and each refusal explains itself (unknown stage, skipped by this workflow, `done`, leaving Planning for Ready, workflow already under way).
- The brief of the thread's agent says the thread's workflow and stage, lists the workflows while it can still choose one, and says when to report a stage:
  - Talking: report `planning` once you start working out how, or `implementing` straight away on the small workflow.
  - Planning: ask the person to approve the plan with `teryon thread ask`; report `implementing` only once they agree.
  - Implementing: report `review` once the change is complete and its tests pass (or `ready` on a workflow without review).
  - In review, when the person writes to the author: report `implementing` if you go back to work on the change.
  - Any stage: when the conversation has moved on to another stage and you have not reported it, report it now.

**Automatic starts**

- When a turn ends and the thread's stage changed during it, Teryon picks the new stage's provider and model:
  1. The candidates: on *only*, the one provider; on *prefer* and *any*, the providers in `among`, or every provider whose program is found.
  2. For Review, drop the thread's own agent.
  3. Drop providers that have run out (decision 7). On *only*, a provider that ran out is the error from decision 12.
  4. On *prefer*, take the preferred provider if it's still there. Otherwise, or on *any*, keep the thread's earlier pick for this stage if it's still there (decision 9), or else pick one of the rest at random.
  5. The model is that provider's model for the stage's strength, or its default (decision 6).
- Implementing, Talking, Planning: if that agent or model differs from the thread's, Teryon changes the thread's agent (using the existing handover when the program differs) and starts a turn with the message from decision 16.
- Review: Teryon starts or continues a review, as "Ask … to review" does today.
- Ready, Done: nothing starts. The thread is marked unread, as when any turn ends.
- A stage with nothing left to pick from starts nothing and adds an error line saying why: "No provider with usage left can review Claude's change (Grok used up until 14:00)", or the *only* error from decision 12.
- A turn that ends because the provider hit its limit, on a stage set to *any* or *prefer*, starts the same stage on another provider (decision 11).
- The stage line and the agent's summary say how the provider was picked: "In review · Grok · 4.7 · picked at random", "Implementing · Claude · Sonnet · Codex used up, so Claude instead of preferred Codex".
- The round limit follows decision 18. The count lives on the thread and is shown on its card when above 0 ("round 2 of 3").
- Stopping a thread (Esc or the menu) also stops automatic starts until your next message.

**Menu**

- "Use the small workflow" (and every other workflow).
- "Board" (`b`) for the current project, "Board of every project" (`B`).
- The thread's summary line under the reply box starts with its stage: `Implementing · fix-login · 3 files changed`.

**Board**

- Columns follow decision 26. Each column header shows the label and number of threads.
- A card shows: title; agent and model (the reviewer's while a review runs); the running or waiting-for-answers mark; the change's name and files changed; the last review's findings; the round count when above 0. On the every-project board, the project's name.
- Within a column, the most recently active thread comes first.
- Keys: `h`/`l` between columns, `j`/`k` within one, Enter opens the thread, Esc goes back. No key moves a card.
- The board updates as threads change, without reloading.

**Not shown**

No git words on the board or in stage names: "Ready to apply", not "ready to merge", unless you relabel it yourself.

## 4. Quality requirements

- **What failure costs:** high, and for automatic starts close to critical. A wrong stage on the board misleads but loses nothing. An automatic start that loops, or starts the wrong agent, uses up usage limits while you're away, and can send an agent to change files you didn't expect.
- **Unit tests** (complete over inputs):
  - settings: no sections, each section alone, every invalid workflow (order, missing `ready` or `done`, bad first stage, unknown stage, duplicate), missing default workflow, `automatic_rounds` 0; `provider` in each of its three forms and every malformed one, unknown agent, `among` with and without `only`, `strength` each value and an unknown one, each of `provider` and `strength` left out, settings on `ready` or `done`, Implementing and Review both `only` the same agent; strengths for each provider set, partly set, unset, and Claude's built-in ones replaced
  - "usage left": no reading, failed reading, a window reached with a reset in the future, in the past, with no reset time, several windows with one reached
  - picking the provider: each of *any*, *prefer*, *only* × the preferred or only provider available, run out, the thread's author (in review), not installed × an earlier pick for the stage present, run out, absent × `among` set or not; *any* with one, several and no candidates; random choice tested with a seeded generator
  - picking the model: strength set, unset (default model and the line saying so)
  - every stage change rule: each automatic move × each workflow, including a stage the workflow skips (a worktree on the small workflow goes to Implementing; a review finishing on a workflow without review can't happen)
  - review verdicts: 0, more than 0, no count, cut short
  - `teryon thread stage`: every stage × every current stage × author or reviewer × in or out of the workflow
  - `teryon thread workflow`: known, unknown, first stage, later stage, change started
  - you changing the workflow while in a stage the new workflow skips
  - choosing the next agent: same agent and model (nothing starts), same program and different model, different program, nothing left to pick, no setting, a turn ended by a usage limit on each of the three modes
  - the round count: increment, limit reached, reset by your message, reset by stopping, limit 0
  - the messages from decision 16 in every case
  - the brief for every stage, with and without the workflow choice
  - the application: board columns from mixed workflows, card contents, sorting, Done folding, key navigation including empty columns, menu commands
- **Request tests:** start a thread and see its stage; the agent reports a stage and the next agent starts when its turn ends, with the right agent, model and message; a review with findings starts the author, with 0 moves to Ready; a stage on *prefer* whose preferred provider has run out starts another; *only* with a provider that ran out adds the error and starts nothing; a turn ended by a usage limit on *any* restarts on another provider; the round limit stops a loop, using the fake program set to always report findings; a message to the author of a thread stuck in In review lets it report `implementing`; you change the workflow; applying moves to Done and a new message moves back to the first stage; every refusal of both commands.
- **End-to-end:** two tests with the fake program.
  1. Settings give Talking and Implementing different models. The fake agent reports `implementing`; the thread shows the second model on its summary line and "Talking → Implementing" in the thread.
  2. Open the board, find the thread's card in its column, let the fake agent report the next stage, and see the card move to the next column without reloading.
- **Monitoring:** an info line in the log for every automatic start (thread, stage, agent, model, round) and a warning when the round limit stops a thread. On a single machine, the note in the thread and the unread mark are the alert.
- **Conventions:** `lets_expect!`, `thiserror`, imported types, inline documentation. Observables in the application, NgModules, zoneless, bun.

## 5. Risks

1. **Automatic starts spend usage limits while you're away.** The round limit, stopping, and the log line bound this. `automatic_rounds = 0` turns it off entirely.
2. **Agents will forget to report stages, or report them too early.** What Teryon sees for itself still moves the main steps (change started, review ended, change applied), and you can move any card. Planning is the weak point: an agent that reports `implementing` without your approval starts the implementing agent. The brief and the `teryon thread ask` step are the only protection; watch for this in the first week.
3. **Handover between programs loses detail.** Codex implementing after Claude planned gets a written account of the conversation, not the conversation. The plan itself is in the thread, which is what matters most. If it turns out too lossy, a later plan can have the planner write the plan to a file the implementer reads.
4. **Findings going straight to the author changes how reviews work.** You no longer filter them. The reviewer brief already asks it to leave out matters of taste; if authors start chasing weak findings, a per-stage switch to wait for you after a review is the fix (left out for now).
5. **The stage count may need to grow** (for example "testing" or "waiting for someone else"). Adding a stage is a code change, deliberately. Keep the internal names stable for the later GitHub mapping.
6. **Usage readings are only as fresh as each provider's last turn.** A provider that ran out yesterday and has since reset looks used up until its reset time passes, which Teryon checks; one that ran out without a reset time looks used up until it runs again. *Prefer* and *any* then skip it longer than needed. You can still choose it yourself, and its next turn refreshes the reading.
7. **Random review picks make reviews less comparable.** Decision 9 keeps the same reviewer for a thread's later rounds, so a thread's reviews stay with one reviewer.
8. **Threads stored before this plan** have no stage. On reading they get the default workflow and a stage worked out from what they have: Done if finished, In review if a reviewer is reviewing, Implementing if they have a change, Talking otherwise.

## 6. Out of scope

- Agents per stage for one workflow only (`[workflows.small.stages.implementing]`)
- Waiting for you between particular stages, as a setting
- New stages, or stages you define yourself
- Mirroring stages to GitHub Projects or a Teryon server
- Dragging cards with the mouse (keys only; the menu works with the mouse)
- Filtering or searching the board
- Effort levels per stage
- Starting a thread again by itself when an *only* provider's limit resets
- Weighting *any* by how much usage each provider has left (it picks evenly)
- A plan file handed between planner and implementer

## 7. Complexity

Large, but in three parts that can land one after another:

1. **Stages and workflows** (settings, thread fields, the two commands, Teryon's own moves, the stage line, the menu). Medium.
2. **Providers and strengths per stage, and automatic starts** (strengths, the three provider modes, usage left, the messages, the round limit, restarting after a usage limit). Medium to large, with the most risk, in the code that runs when a turn ends.
3. **The board** (a new screen). Medium, almost all in the application.

Each part is useful alone: part 1 gives stages, part 1 and 3 give a board without automation.

## 8. Implementation details

### Data (`core/src/store/mod.rs`)

- `enum Stage { Talking, Planning, Implementing, Review, Ready, Done }`, in the fixed order, serialised lowercase.
- `Thread`:
  - `workflow: String`
  - `stage: Stage`
  - `rounds: u32`, automatic starts since your last message
  - `held: bool`, set by stopping, cleared by your message, so automatic starts wait
  - `picks: BTreeMap<Stage, String>`, the provider picked for each stage, so it sticks (decision 9)
  - all `#[serde(default)]`; the store fills `workflow` and `stage` for old threads as in risk 6
- A `stage` entry kind: `{ "from": "talking", "to": "implementing", "by": "agent" | "person" | "teryon", "agent": "claude", "reason": "review" }`.

### Service

- `core/src/settings.rs`:
  - `WorkflowSettings { default, automatic_rounds }`
  - `workflows: BTreeMap<String, WorkflowDefinition { description, stages }>`
  - `stages: StageSettings` with one optional `StageSetting { provider: Option<ProviderChoice>, among: Option<Vec<Agent>>, strength: Option<Strength>, label: Option<String> }` per stage
  - `ProviderChoice { Any, Prefer(Agent), Only(Agent) }` read from `"any"`, `"prefer <agent>"`, `"only <agent>"`; `Strength { Strong, Medium, Fast }`
  - `strong`, `medium`, `fast: Option<String>` on `ClaudeSettings`, `CodexSettings`, `GrokSettings`
  - checks from section 3; update the example at the top of the file
- `core/src/workflow/mod.rs` (new): the rules, kept free of storage so they're easy to test.
  - `Workflows` built from the settings and the built-in ones
  - `fn moved_by_teryon(thread, event) -> Option<Stage>` for the events in decision 20
  - `fn reported(thread, stage, voice) -> Result<Stage, StageError>` for `teryon thread stage`
  - `fn picked(thread, name) -> Result<(), StageError>` for `teryon thread workflow`
  - `fn has_usage_left(limits: Option<&Limits>, now) -> bool`, from the readings in `core/src/usage`
  - `fn pick(stage_setting, thread, available: &[Agent], usage, random: &mut impl Rng) -> Result<Pick, PickError>`, where `Pick { agent, model, why }` carries the reason shown on the stage line. The random generator is passed in so tests can seed it.
  - `fn next_agent(thread, stage, settings, pick) -> Next` with `Next::Nothing`, `Next::Author { agent, model, message }`, `Next::Review { agent, model }`, `Next::Refused(PickError)`, `Next::Held`
  - `StageError` with `thiserror`
- `core/src/agents/staging.rs` (new): the brief text per stage and the workflow list, next to `naming.rs` and `asking.rs`, joined into the brief the same way.
- `core/src/protocol.rs`:
  - `SetStage { thread_id, turn, stage }`, sent only by `teryon thread stage`
  - `SetWorkflow { thread_id, turn: Option<Uuid>, workflow }`
  - `Snapshot` gets `workflows` and the stage labels
  - regenerate the TypeScript
- `core/src/workspace/mod.rs`:
  - `set_stage`, `set_workflow`, recording the `stage` entry
  - Teryon's own moves in `start_thread`, `start_change`, `review_change`, the review verdict in the `ThreadChange::Reviewed` arm, and in `changes.rs` after apply, discard and acceptance on GitHub
  - `send_message` resets `rounds` and `held`, and moves a Done thread to the first stage
  - `stop` sets `held`
  - in the `ThreadChange::Finished` arm: when the stage changed during the turn, ask `next_agent` and act. Reuse `change_agent` and `review_change` rather than starting turns another way, so handover and the review checks stay in one place.
- `service/src/session.rs`: route the two requests. The `teryon` command line: `thread stage` and `thread workflow` subcommands next to `thread title`.

### Application (`app/src/app/workspace/`)

- `workspace.model.ts`: `stageLabel`, the stage in `summaryText`, the `stage` entry's line in the thread.
- `board.model.ts` (new): columns from threads and workflows, card contents, sorting, folding Done, key handling. Pure functions over observables, tested in `board.model.spec.ts`.
- `board.component.ts` (new): the screen, in `WorkspaceModule`, shown through `showScreen` like Files and History.
- `commands.model.ts`: "Use the … workflow", "Board", "Board of every project".
- `workspace.api.ts`: the two requests.
- `e2e/workspace.spec.ts`, `e2e/support.ts`: the two tests. The fake program learns to run `teryon thread stage`.

### Steps

Each step: write the failing tests, make them pass, commit.

1. `Stage`, the thread fields and filling old threads.
2. Settings: workflows, stages, choices, checks.
3. `workflow` rules: Teryon's moves, reports, workflow picks.
4. The two commands, the brief, the `stage` entry, the protocol; request tests.
5. Stage in the summary line, the stage line in the thread, the menu commands.
6. Strengths in settings and the menu; `has_usage_left` and `pick`.
7. `next_agent`, the messages, automatic starts at the end of a turn, restarting after a usage limit, the round limit and holding; request tests including the loop.
8. The board model, then the board screen and its keys.
9. The end-to-end tests. Then run Teryon with the settings from section 3, start a thread, approve a plan, and check that Codex implements, a reviewer is picked from Claude and Grok, findings go back, and the board follows each step.
