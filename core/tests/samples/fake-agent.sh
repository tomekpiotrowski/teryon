#!/bin/sh
# Stands in for the claude, codex and grok programs in tests.
#
# Tests must never reach a real provider, so the test configuration starts
# this instead. It replays recordings of the real programs and can be told,
# through words in the message, to misbehave in the ways Teryon has to handle.
#
# It appends how it was started, on one line, to `.fake-agent-invocations` in
# the directory it runs in, which is the project directory, so tests can check
# that too.

agent="$1"
shift

# Grok's version, which Teryon asks after every Grok turn to tell Grok's
# servers. It is not a turn, so it is not noted below.
if [ "$1" = "--version" ]; then
  echo "grok 1.0.92 (b06756fd6256) [alpha]"
  exit 0
fi
samples="$(dirname "$0")/agents"

# `grok models`, asked once when the service starts, outside any project:
# a recording of the list it prints, and nothing written anywhere.
if [ "$*" = "models" ]; then
  cat "$samples/$agent-models.txt"
  exit 0
fi
# Codex's model discovery handshake. Check the request order so tests catch
# a client that asks before initialization or forgets the next page.
if [ "$agent" = "codex" ] && [ "$1" = "app-server" ]; then
  IFS= read -r request
  case "$request" in *'"method":"initialize"'*) ;; *) exit 2 ;; esac
  echo '{"id":0,"result":{}}'
  IFS= read -r request
  case "$request" in *'"method":"initialized"'*) ;; *) exit 2 ;; esac
  IFS= read -r request
  case "$request" in *'"method":"model/list"'*) ;; *) exit 2 ;; esac
  echo '{"method":"notice","params":{}}'
  echo '{"id":1,"result":{"data":[{"id":"display-id","model":"gpt-6-astra"},{"model":"hidden-model","hidden":true}],"nextCursor":"page-two"}}'
  IFS= read -r request
  case "$request" in *'"cursor":"page-two"'*) ;; *) exit 2 ;; esac
  echo '{"id":2,"result":{"data":[{"model":"gpt-6-astra"},{"model":"gpt-6-sol"}],"nextCursor":null}}'
  # Stay alive until the client closes its input or stops the process.
  cat >/dev/null
  exit 0
fi
# Where the project is: a reviewer runs inside a thread's worktree, and
# tests look for what it leaves in the project directory.
common=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null)
case "$common" in
  */.git) project="${common%/.git}" ;;
  *) project="." ;;
esac
printf '%s\n' "$(printf '%s' "$*" | tr '\n' ' ')" >> "$project/.fake-agent-invocations"

# The word after `$1` in the message, such as the name in "start a change fix-login".
word_after() {
  printf '%s\n' "$*" | tr '\n' ' ' | sed -n "s/.*$1 \([a-z0-9-]*\).*/\1/p"
}

# Starts a change through Teryon, as agents are told to, and leaves an
# uncommitted file in it. What `teryon` printed, on either output, goes to
# `.fake-agent-worktree`, so tests can see the path or the refusal.
start_change() {
  name=$(word_after "start a change" "$*")
  if path=$(teryon worktree create "$name" 2>.fake-agent-worktree-error); then
    printf '%s\n' "$path" > .fake-agent-worktree
    printf 'made by the fake agent\n' > "$path/$name.txt"
  else
    cat .fake-agent-worktree-error > .fake-agent-worktree
  fi
  rm -f .fake-agent-worktree-error
}

# Names the thread through Teryon, as agents are told to, with the text in
# quotes after "name the thread". What `teryon` printed on errors goes to
# `.fake-agent-title`, empty when it agreed.
name_thread() {
  title=$(printf '%s\n' "$*" | tr '\n' ' ' | sed -n 's/.*name the thread "\([^"]*\)".*/\1/p')
  teryon thread title "$title" 2>.fake-agent-title
  printf '\n' >> .fake-agent-title
}

# Asks the person questions through Teryon, as agents are told to. The
# questions are `.fake-agent-questions` in the project when a test put one
# there, and two questions otherwise: one taking one answer, one several.
# What `teryon` printed, on either output, goes to `.fake-agent-asked`.
ask_questions() {
  if [ -f "$project/.fake-agent-questions" ]; then
    teryon thread ask < "$project/.fake-agent-questions" > .fake-agent-asked 2>&1
  else
    teryon thread ask > .fake-agent-asked 2>&1 <<'EOF'
{"questions": [
  {"question": "Where should answers be kept?", "answers": [{"label": "In the thread", "description": "An ordinary message"}, {"label": "On the record"}]},
  {"question": "Which screens should show it?", "answers": [{"label": "Thread list"}, {"label": "Window title"}, {"label": "Notification"}], "several": true, "recommended": [0, 2]}
]}
EOF
  fi
  printf '\n' >> .fake-agent-asked
}

# Makes a worktree by hand, ignoring the instruction, as an agent might.
make_worktree() {
  name=$(word_after "make a worktree called" "$*")
  git worktree add -q -b "$name" ".teryon/worktrees/$name" HEAD
}

# Notes the thread and turn Teryon started it for, and where `teryon` is.
remember_turn() {
  printf '%s %s %s %s\n' "$TERYON_THREAD" "$TERYON_TURN" "${TERYON_DATA:-none}" "$(command -v teryon || echo none)" > .fake-agent-turn
}

# Reviews as a reviewer is asked to. With `.fake-reviewer-edits` in the
# project, it also edits `login.txt` in the worktree it runs in, as a
# reviewer must not. With `.fake-reviewer-slow`, it starts and then waits, to be stopped.
review() {
  if [ -f "$project/.fake-reviewer-edits" ]; then
    printf 'edited by the reviewer\n' >> login.txt
  fi
  if [ -f "$project/.fake-reviewer-slow" ]; then
    head -n 1 "$samples/$agent-first.jsonl"
    exec sleep 30
  fi
  cat "$samples/$agent-$1.jsonl"
}

case "$*" in
  *"Review the change."*)
    review review "$*"
    ;;
  *"Review it again"*)
    review review-again "$*"
    ;;
  *"please fail"*)
    echo "the fake agent was told to fail" >&2
    exit 3
    ;;
  *"end silently"*)
    exit 4
    ;;
  *"be slow"*)
    head -n 1 "$samples/$agent-first.jsonl"
    exec sleep 30
    ;;
  *"be stubborn"*)
    trap '' INT
    head -n 1 "$samples/$agent-first.jsonl"
    exec sleep 30
    ;;
  *"say nonsense"*)
    echo "this is not a report line"
    cat "$samples/$agent-first.jsonl"
    ;;
  *"use tools"*)
    cat "$samples/$agent-tools.jsonl"
    ;;
  *"start a change"*)
    start_change "$*"
    cat "$samples/$agent-first.jsonl"
    ;;
  *"ask questions slowly"*)
    # Asks a moment after starting, so a test can be doing something else when the questions arrive.
    head -n 1 "$samples/$agent-first.jsonl"
    sleep 2
    ask_questions
    tail -n +2 "$samples/$agent-first.jsonl"
    ;;
  *"ask questions"*)
    ask_questions
    cat "$samples/$agent-first.jsonl"
    ;;
  *"name the thread"*)
    name_thread "$*"
    cat "$samples/$agent-first.jsonl"
    ;;
  *"make a worktree called"*"and wait"*)
    make_worktree "$*"
    head -n 1 "$samples/$agent-first.jsonl"
    exec sleep 30
    ;;
  *"make a worktree called"*)
    make_worktree "$*"
    cat "$samples/$agent-first.jsonl"
    ;;
  *"remember the turn and wait"*)
    remember_turn
    head -n 1 "$samples/$agent-first.jsonl"
    exec sleep 30
    ;;
  *"write markdown"*)
    # Not a recording as such: the first one with the answer replaced by hand
    # with one that uses every kind of markdown the thread has to show, and
    # some it must refuse to act on.
    cat "$samples/$agent-markdown.jsonl"
    ;;
  *)
    cat "$samples/$agent-first.jsonl"
    ;;
esac
