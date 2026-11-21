// Generated from core/src/protocol.rs by `UPDATE_PROTOCOL=1 cargo test -p teryon-core protocol`.
// Do not edit by hand.

export type JsonValue = number | string | boolean | null | JsonValue[] | { [key: string]: JsonValue };

export type ClientMessage = { "type": "hello", token: string, } | { "type": "call", id: number, request: Request, };

export type Request = { "type": "add_project", name: string, directory: string, } | { "type": "directories", path: string, } | { "type": "remove_project", project_id: string, } | { "type": "start_thread", project_id: string, agent: string, 
/**
 * Nothing leaves the model to the agent program.
 */
model: string | null, title: string, } | { "type": "send_message", thread_id: string, text: string, to: Voice, } | { "type": "review_change", thread_id: string, 
/**
 * The agent that reviews, which may not be the thread's own.
 */
agent: string, 
/**
 * Nothing leaves the model to the agent program.
 */
model: string | null, } | { "type": "change_agent", thread_id: string, to: Voice, agent: string, 
/**
 * Nothing leaves the model to the agent program.
 */
model: string | null, } | { "type": "stop", thread_id: string, } | { "type": "mark_seen", thread_id: string, } | { "type": "start_change", thread_id: string, turn: string, name: string, from: string | null, } | { "type": "name_thread", thread_id: string, turn: string, title: string, } | { "type": "draft_description", thread_id: string, } | { "type": "apply_change", thread_id: string, description: string, } | { "type": "catch_up", thread_id: string, } | { "type": "discard_change", thread_id: string, } | { "type": "send_for_review", thread_id: string, description: string, } | { "type": "open_review", thread_id: string, } | { "type": "open_copy", thread_id: string, with: Opener, } | { "type": "follow_thread", thread_id: string, } | { "type": "unfollow_thread", thread_id: string, } | { "type": "files", project_id: string, thread_id: string | null, branch: string | null, commit: string | null, } | { "type": "file", project_id: string, thread_id: string | null, branch: string | null, commit: string | null, path: string, } | { "type": "history", project_id: string, branch: string | null, } | { "type": "changes", project_id: string, commit: string, from: string | null, } | { "type": "change", project_id: string, commit: string, from: string | null, path: string, } | { "type": "usage" } | { "type": "shutdown" };

export type ServiceMessage = { "type": "reply", id: number, outcome: Outcome, } | { "type": "event", event: Event, };

export type Outcome = { "type": "done", answer: Answer, } | { "type": "failed", message: string, };

export type Answer = { "type": "nothing" } | { "type": "project", project: ProjectView, } | { "type": "directories", listing: DirectoryListing, } | { "type": "thread", thread: Thread, } | { "type": "entry", entry: Entry, } | { "type": "entries", entries: Array<Entry>, } | { "type": "listing", listing: Listing, } | { "type": "file", file: FileView, } | { "type": "history", history: History, } | { "type": "changes", changes: Changes, } | { "type": "path", path: string, } | { "type": "draft", text: string, } | { "type": "caught_up", overlaps: boolean, } | { "type": "usage", report: UsageReport, };

export type Event = { "type": "snapshot", projects: Array<ProjectView>, threads: Array<Thread>, opens: Opens, 
/**
 * The models each agent offers besides its default.
 */
models: ModelLists, } | { "type": "models_changed", models: ModelLists, } | { "type": "project_added", project: ProjectView, } | { "type": "project_removed", project_id: string, } | { "type": "thread_changed", thread: Thread, } | { "type": "entry_added", entry: Entry, } | { "type": "limits_changed", limits: Limits, } | { "type": "usage_recorded", line: UsageLine, };

export type ProjectView = { id: string, name: string, directory: string, created_at: string, state: ProjectState, 
/**
 * Whether its `origin` is on GitHub, so changes can be sent for review there.
 */
on_github: boolean, };

export type ProjectState = "ready" | "missing" | "in_use";

export type DirectoryListing = { 
/**
 * The real directory whose children these are.
 */
directory: string, 
/**
 * That directory's parent. Nothing at the top of the filesystem.
 */
parent: string | null, 
/**
 * The child directories, by name, in case-insensitive order.
 *
 * Names starting with `.` are included. The screen hides them until
 * what is typed starts with `.`, so a home folder is not a wall of
 * configuration.
 */
children: Array<string>, 
/**
 * What was typed after `directory`. Empty when the typed path is this directory.
 */
remainder: string, };

export type Thread = { id: string, 
/**
 * The project the thread belongs to.
 *
 * Not trusted from the file: a project removed and added again gets a
 * new identifier, and its threads follow it. The store sets it on reading.
 */
project_id: string, title: string, 
/**
 * Whether the title was chosen, by the person or by the agent.
 *
 * Until then the title is the start of the first message, which often
 * tells threads apart poorly, and the agent is asked to name the thread.
 */
named: boolean, agent: string, 
/**
 * The model the agent runs on. Nothing leaves it to the agent program's own settings.
 *
 * Can change between messages; each turn's summary says which model did that turn.
 */
model: string | null, 
/**
 * The identifier the agent program gave this conversation.
 *
 * Nothing until the program has started it: the program makes the
 * identifier up and reports it, and the next message continues it.
 */
conversation: string | null, 
/**
 * Whether the agent took the thread over from another agent and has not yet been told what was said.
 *
 * Its next message carries the thread so far, see
 * `crate::agents::handover`. Cleared once its program names the new
 * conversation, which then holds it.
 */
handover: boolean, status: Status, 
/**
 * Whether an agent stopped working here since the person last looked.
 */
unread: boolean, 
/**
 * The slash commands the agent program last said it takes.
 *
 * Empty until the program has run once for this thread.
 */
commands: Array<string>, 
/**
 * The change the thread's agent is working on, in a copy of the project of its own.
 *
 * Nothing until the agent starts one. Threads stored before changes had
 * names have none either, and their copy is taken over, see
 * `crate::changes::claim`. Boxed, because it is large next to the rest,
 * and a thread travels in most messages.
 */
change: Change | null, 
/**
 * How the thread's last change ended. Nothing while one is under way, or before the first.
 */
finished: Finish | null, 
/**
 * The second agent, reviewing the change. Nothing until a review is asked for.
 */
reviewer: Reviewer | null, created_at: string, };

export type Status = "idle" | "running";

export type Change = { 
/**
 * The worktree folder's name in `.teryon/worktrees/`, also its branch.
 */
name: string, 
/**
 * The worktree's full path.
 */
path: string, 
/**
 * The branch the change started from, and is applied to. Nothing when
 * the project was not on a branch when the change started.
 */
base: string | null, 
/**
 * The commit the change started from.
 */
start: string, 
/**
 * The branch checked out in the worktree now, read from git. Usually `name`.
 */
branch: string | null, 
/**
 * Nothing until first read.
 */
state: ChangeState | null, 
/**
 * The review the change was sent for on GitHub. Nothing until it is sent.
 */
review: Review | null, };

export type ChangeState = { 
/**
 * Whether the worktree is still there.
 */
copy_present: boolean, 
/**
 * Whether the branch the change started from is still there.
 */
base_present: boolean, 
/**
 * Files differing between the worktree as it is now and where the change
 * started, whether the agent committed them or not.
 */
files_changed: number, fit: Fit, 
/**
 * The agent left the worktree in the middle of a rebase or a merge, or
 * switched it to another branch. Something only the agent can sort out.
 */
unfinished: boolean, };

export type Review = { number: number, 
/**
 * Where the review is on GitHub, for the person to open.
 */
url: string, state: ReviewState, };

export type ReviewState = "in_review" | "accepted" | "declined";

export type Finish = "applied" | "accepted" | "discarded" | "removed";

export type Reviewer = { agent: string, 
/**
 * The model the reviewer runs on. Nothing leaves it to the agent program's own settings.
 */
model: string | null, 
/**
 * The identifier the agent program gave the reviewer's conversation. Nothing until it has started it.
 */
conversation: string | null, 
/**
 * Whether the reviewer took over from another reviewer and has not yet been told what was said.
 */
handover: boolean, 
/**
 * Whether the turn running in the thread is the reviewer's.
 */
reviewing: boolean, 
/**
 * What the last review found. Nothing until one has ended.
 */
verdict: Verdict | null, };

export type Verdict = { 
/**
 * How many problems the reviewer counted. Nothing when it did not say.
 */
findings: number | null, 
/**
 * Whether the author has worked on the change since, so the review may no longer fit it.
 */
answered: boolean, 
/**
 * Whether the review ended before the reviewer finished it, so it may be missing findings.
 */
cut: boolean, };

export type Voice = "author" | "reviewer";

export type Fit = "applies" | "overlaps" | "unknown";

export type Opener = "folder" | "editor" | "terminal";

export type Opens = { editor: boolean, terminal: boolean, };

export type ModelLists = { claude: Array<string>, codex: Array<string>, grok: Array<string>, };

export type Entry = { id: string, thread_id: string, 
/**
 * Order within the thread, starting at 1.
 */
position: number, kind: string, body: JsonValue, created_at: string, };

export type Listing = { branches: Array<string>, 
/**
 * The branch shown. Nothing when the project's directory is not on a
 * branch, and when a commit is shown.
 */
branch: string | null, 
/**
 * The commit shown, by its full name. Nothing unless a commit was asked for.
 */
commit: string | null, 
/**
 * The branch the changes are counted from.
 *
 * Nothing when the branch shown is that branch itself, or when there is
 * no such branch. Then the only changes are the ones not committed yet.
 */
base: string | null, files: Array<FileEntry>, };

export type FileEntry = { 
/**
 * Where the file is, from the top of the repository, with `/` between folders.
 */
path: string, 
/**
 * Nothing for a file that did not change.
 */
change: FileChange | null, };

export type FileChange = "added" | "modified" | "deleted";

export type FileView = { path: string, content: Content, };

export type Content = { "kind": "text", lines: Array<Line>, } | { "kind": "binary" } | { "kind": "too_large" };

export type Line = { 
/**
 * Nothing for a line that is the same in both versions.
 */
change: LineChange | null, 
/**
 * Where the line is in the file as it is now. A removed line has no place.
 */
number: number | null, text: string, };

export type LineChange = "added" | "removed";

export type History = { branches: Array<string>, 
/**
 * The branch shown. Nothing when the project's directory is not on a branch.
 */
branch: string | null, 
/**
 * The main branch, when the branch shown is another one.
 *
 * Nothing when the branch shown is the main branch itself, or when
 * there is no main branch.
 */
base: string | null, 
/**
 * How many commits the branch shown has that the main branch has not.
 */
ahead: number, 
/**
 * Whether the branch has older commits than the ones listed.
 */
more: boolean, commits: Array<Commit>, };

export type Commit = { 
/**
 * The commit's full name.
 */
hash: string, 
/**
 * The commits this one was made on top of, the one it continues first.
 */
parents: Array<string>, 
/**
 * The first line of the message.
 */
subject: string, 
/**
 * The name of the person the commit is recorded under.
 */
author: string, email: string, 
/**
 * When the commit came to be, in seconds since 1970.
 *
 * This is what git calls the committer date. Unlike the author date it
 * moves when a commit is made again by a rebase or a squash, which is
 * what tells the commit apart from the work that was done before.
 */
time: number, 
/**
 * Whether the main branch has this commit too.
 */
on_base: boolean, 
/**
 * The thread whose agent was working when the commit was made, see `turns`.
 */
thread: string | null, 
/**
 * The agent that was working then, see `turns`. Nothing for a commit made by hand.
 *
 * Not the thread's agent now: a thread can move to another agent, and
 * earlier commits stay the earlier agent's.
 */
agent: string | null, 
/**
 * The files the commit changed, each from the top of the repository.
 */
files: Array<string>, };

export type Changes = { 
/**
 * The commit looked at, by its full name.
 */
commit: string, 
/**
 * The commit it is compared with. Nothing for a commit without a parent.
 */
from: string | null, 
/**
 * The commit's message after its first line.
 */
body: string, files: Array<ChangedFile>, };

export type ChangedFile = { path: string, change: FileChange, 
/**
 * How many lines were added and removed. Nothing for a file that is not text.
 */
added: number | null, removed: number | null, };

export type UsageReport = { 
/**
 * Every agent's latest limits, in the order the application offers agents. Agents never read are left out.
 */
limits: Array<Limits>, 
/**
 * The turns of the last 14 days, oldest first.
 */
lines: Array<UsageLine>, };

export type Limits = { 
/**
 * The agent's name, as on a thread.
 */
agent: string, 
/**
 * The subscription, such as "Plus", when the program names it.
 */
plan: string | null, 
/**
 * Shortest first.
 */
windows: Array<Window>, 
/**
 * When the program reported them. They are only as fresh as the agent's last turn.
 */
taken_at: string, 
/**
 * Whether reading them after the last turn failed, so they may be older than that turn.
 */
failed: boolean, 
/**
 * The program's own answer, kept whole while Teryon cannot read it yet.
 *
 * Grok will say its limits outside its window one day, in a form not
 * known today. Keeping what it says is what makes reading it possible later.
 */
raw: JsonValue | null, };

export type Window = { 
/**
 * What the program calls it, such as `five_hour` or `primary`.
 */
name: string, 
/**
 * What a person calls it, such as "5 hours".
 */
label: string, 
/**
 * How long the window is, when known. Windows are shown shortest first.
 */
minutes: number | null, 
/**
 * How much of it is used, from 0 to 100.
 */
used: number, 
/**
 * When it starts over. Nothing when the program did not say.
 */
resets_at: string | null, 
/**
 * Whether the program said it is used up, so the agent cannot work until it resets.
 */
reached: boolean, };

export type UsageLine = { 
/**
 * When the turn ended.
 */
at: string, agent: string, 
/**
 * The model that ran, when the program said. Nothing means the program's own default.
 */
model: string | null, project_id: string, 
/**
 * The project's name when the turn ran, for when the project has since been removed.
 */
project_name: string, thread_id: string, read: number, written: number, };
