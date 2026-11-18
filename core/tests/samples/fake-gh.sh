#!/bin/sh
# Stands in for GitHub's `gh` program in tests.
#
# Tests must never reach GitHub, so the test configuration starts this
# instead. It appends how it was started, on one line, to
# `.fake-gh-invocations` in the directory it runs in, which is the project.
#
# What it answers is arranged through files there:
# - `.fake-gh-signed-out` present: it says the person must sign in, as gh does.
# - `.fake-gh-review` holds the review's state: OPEN, MERGED or CLOSED. OPEN when absent.

printf '%s\n' "$*" >> .fake-gh-invocations

if [ -e .fake-gh-signed-out ]; then
  echo "To get started with GitHub CLI, please run:  gh auth login" >&2
  exit 4
fi

url="https://github.com/owner/app/pull/12"
case "$1 $2" in
  "pr create")
    echo "$url"
    ;;
  "pr view")
    state=$(cat .fake-gh-review 2>/dev/null || echo OPEN)
    printf '{"number":12,"url":"%s","state":"%s"}\n' "$url" "$state"
    ;;
  *)
    echo "the fake gh does not know: $*" >&2
    exit 1
    ;;
esac
