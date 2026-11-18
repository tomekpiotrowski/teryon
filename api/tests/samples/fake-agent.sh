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

# Creates the worktree the instruction names, on the branch `feature`.
# The path is taken out of the arguments, punctuation and all.
make_worktree() {
  path=$(printf '%s\n' "$1" | sed -n 's/.*\(\.teryon\/worktrees\/[0-9a-f][-0-9a-f]*\).*/\1/p')
  if [ -n "$path" ]; then
    git worktree add -q -b feature "$path" HEAD
  fi
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
  *"make a worktree and wait"*)
    make_worktree "$*"
    head -n 1 "$samples/$agent-first.jsonl"
    exec sleep 30
    ;;
  *"make a worktree"*)
    make_worktree "$*"
    cat "$samples/$agent-first.jsonl"
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
