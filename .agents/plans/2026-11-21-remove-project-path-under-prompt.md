# Remove the project path under the prompt

## 1. Overview

When a project is open and no thread is chosen, the screen shows the "What should Claude do in teryon?" box. The row under it shows the agent choice, then the project's full path (for example `/home/tomek/Projects/teryon`), then the menu, history, code and Run buttons.

The path takes up room and repeats what is already on screen: the project name is in the prompt text, and the full path was shown when the project was added. This plan removes it.

Before:

```
claude · opus                 /home/tomek/Projects/teryon   menu /   history H   code C   Run ↵
```

After:

```
claude · opus                                               menu /   history H   code C   Run ↵
```

## 2. What gets lost

Two projects with the same name in different folders now look the same on this screen. That is accepted: project names can be changed when a project is added, and the menu still lists projects by name.

## 3. Change

One file, `app/src/app/workspace/workspace.page.html`, inside `.composer-meta` of the new thread screen. Remove:

```html
<span>{{ view.project.directory }}</span>
```

Nothing else changes:

- The `.spacer` before it already pushes the buttons to the right, so the layout stays the same.
- `.composer-meta` has no rule aimed at a plain `span`, so no styles become unused.
- The thread screen's reply box (`.thread-composer`) does not show the path, so it needs no change.
- `project.directory` is still used elsewhere (the code screen, adding a project), so the data stays.

## 4. Tests

The cost of failure is low: if the path came back, nothing would break.

- **Unit tests:** none to add. `workspace.page.spec.ts` replaces the page's template with an empty one and only tests what the page decides, not what it shows. Nothing there refers to the path.
- **End-to-end tests:** none to add. No test in `e2e/` looks for the path, and a browser test that checks for something missing costs more to run than it is worth here.
- **Run before finishing:** in `app/`, `bun run lint`, `bunx ng test --watch=false` and `bun run build`; in `e2e/`, the full Playwright suite, to confirm the new thread screen still works.

## 5. Checking by hand

Open a project with no thread chosen. The row under the prompt shows the agent choice on the left and the buttons on the right, with no path between them.
