#!/bin/sh
# Stands in for the claude and grok programs in tests.
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
samples="$(dirname "$0")/agents"
printf '%s\n' "$(printf '%s' "$*" | tr '\n' ' ')" >> .fake-agent-invocations

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

# Makes a worktree by hand, ignoring the instruction, as an agent might.
make_worktree() {
  name=$(word_after "make a worktree called" "$*")
  git worktree add -q -b "$name" ".teryon/worktrees/$name" HEAD
}

# Notes the thread and turn Teryon started it for, and where `teryon` is.
remember_turn() {
  printf '%s %s %s %s\n' "$TERYON_THREAD" "$TERYON_TURN" "${TERYON_DATA:-none}" "$(command -v teryon || echo none)" > .fake-agent-turn
}

case "$*" in
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
