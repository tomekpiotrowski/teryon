// Generated from core/src/protocol.rs by `UPDATE_PROTOCOL=1 cargo test -p teryon-core protocol`.
// Do not edit by hand.

export type JsonValue = number | string | boolean | null | JsonValue[] | { [key: string]: JsonValue };

export type ClientMessage = { "type": "hello", token: string, } | { "type": "call", id: number, request: Request, };

export type Request = { "type": "add_project", name: string, directory: string, } | { "type": "remove_project", project_id: string, } | { "type": "start_thread", project_id: string, agent: string, title: string, } | { "type": "send_message", thread_id: string, text: string, } | { "type": "stop", thread_id: string, } | { "type": "mark_seen", thread_id: string, } | { "type": "start_change", thread_id: string, turn: string, name: string, from: string | null, } | { "type": "draft_description", thread_id: string, } | { "type": "apply_change", thread_id: string, description: string, } | { "type": "catch_up", thread_id: string, } | { "type": "discard_change", thread_id: string, } | { "type": "open_copy", thread_id: string, with: Opener, } | { "type": "follow_thread", thread_id: string, } | { "type": "unfollow_thread", thread_id: string, } | { "type": "files", project_id: string, branch: string | null, commit: string | null, } | { "type": "file", project_id: string, branch: string | null, commit: string | null, path: string, } | { "type": "history", project_id: string, branch: string | null, } | { "type": "changes", project_id: string, commit: string, from: string | null, } | { "type": "change", project_id: string, commit: string, from: string | null, path: string, } | { "type": "shutdown" };

export type ServiceMessage = { "type": "reply", id: number, outcome: Outcome, } | { "type": "event", event: Event, };

export type Outcome = { "type": "done", answer: Answer, } | { "type": "failed", message: string, };

export type Answer = { "type": "nothing" } | { "type": "project", project: ProjectView, } | { "type": "thread", thread: Thread, } | { "type": "entry", entry: Entry, } | { "type": "entries", entries: Array<Entry>, } | { "type": "listing", listing: Listing, } | { "type": "file", file: FileView, } | { "type": "history", history: History, } | { "type": "changes", changes: Changes, } | { "type": "path", path: string, } | { "type": "draft", text: string, } | { "type": "caught_up", overlaps: boolean, };

export type Event = { "type": "snapshot", projects: Array<ProjectView>, threads: Array<Thread>, opens: Opens, } | { "type": "project_added", project: ProjectView, } | { "type": "project_removed", project_id: string, } | { "type": "thread_changed", thread: Thread, } | { "type": "entry_added", entry: Entry, };

export type ProjectView = { id: string, name: string, directory: string, created_at: string, state: ProjectState, };

export type ProjectState = "ready" | "missing" | "in_use";

export type Thread = { id: string, 
/**
 * The project the thread belongs to.
 *
 * Not trusted from the file: a project removed and added again gets a
 * new identifier, and its threads follow it. The store sets it on reading.
 */
project_id: string, title: string, agent: string, 
/**
 * The identifier the agent program knows this conversation by.
 */
conversation_id: string, 
/**
 * Whether the agent program has seen the conversation yet.
 *
 * The programs take a different option for starting a conversation than
 * for continuing one, and refuse the wrong one.
 */
conversation_started: boolean, status: Status, 
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
change: Change | null, created_at: string, };

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
state: ChangeState | null, };

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

export type Fit = "applies" | "overlaps" | "unknown";

export type Opener = "folder" | "editor" | "terminal";

export type Opens = { editor: boolean, terminal: boolean, };

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
