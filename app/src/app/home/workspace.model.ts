export interface ProjectItem {
  readonly glyph: string;
  readonly label: string;
}

export interface Project {
  readonly id: string;
  readonly name: string;
  readonly runs: string;
  readonly live: boolean;
  readonly items: readonly ProjectItem[];
}

export interface DiffLine {
  readonly number: string;
  readonly text: string;
  readonly marker: ' ' | '+' | '-';
}

export type ThreadEntry =
  | { readonly kind: 'message'; readonly author: 'user' | 'agent'; readonly text: string; readonly time?: string }
  | { readonly kind: 'activity'; readonly lines: readonly string[] }
  | { readonly kind: 'code'; readonly text: string }
  | { readonly kind: 'diff'; readonly verb: string; readonly file: string; readonly lines: readonly DiffLine[] }
  | {
      readonly kind: 'attention';
      readonly glyph: string;
      readonly text: string;
      readonly primary: string;
      readonly secondary: string;
      readonly time?: string;
    }
  | { readonly kind: 'recap'; readonly text: string }
  | { readonly kind: 'live'; readonly text: string };

export const PROJECTS: readonly Project[] = [
  {
    id: 'payments',
    name: 'Payments migration',
    runs: '4 runs today',
    live: true,
    items: [
      { glyph: '!', label: 'Drop legacy_txn' },
      { glyph: '?', label: 'Currency default?' },
      { glyph: '▤', label: 'Adapter diff' },
    ],
  },
  {
    id: 'onboarding',
    name: 'Onboarding rewrite',
    runs: '2 runs today',
    live: false,
    items: [{ glyph: '✓', label: 'Plan · 9 steps' }],
  },
  { id: 'support', name: 'Support triage', runs: '11 runs today', live: false, items: [] },
  { id: 'docs', name: 'Docs sweep', runs: 'no runs yet', live: false, items: [] },
];

export const INITIAL_THREADS: Readonly<Record<string, readonly ThreadEntry[]>> = {
  payments: [
    {
      kind: 'message',
      author: 'user',
      time: '10:40 PM',
      text: 'Move all payment writes off legacy_txn onto the new ledger. No downtime, and keep the old table readable for 30 days.',
    },
    {
      kind: 'activity',
      lines: [
        'Spawned 3 agents · planner, migrator, verifier',
        'Read 41 files, searched 6 patterns',
        'Thought for 4.4s',
        'Run  Map schema deltas between legacy_txn and ledger',
        'Run  Map schema deltas between legacy_txn and ledger',
        'Thought for 1.2s',
      ],
    },
    {
      kind: 'diff',
      verb: 'Edit',
      file: 'api/src/payments/writer.rs',
      lines: [
        { number: '106', text: '    let tx = ledger::begin(&pool).await?;', marker: ' ' },
        { number: '107', text: '-   legacy_txn::insert(&tx, &payment).await?;', marker: '-' },
        { number: '107', text: '+   ledger::insert(&tx, &payment.into_entry()).await?;', marker: '+' },
        { number: '108', text: '    tx.commit().await?;', marker: ' ' },
      ],
    },
    {
      kind: 'message',
      author: 'agent',
      time: '10:50 PM',
      text: 'The adapter is in. Writes now land in the ledger and legacy_txn is read-only behind a view, so the 30-day window holds without a second code path.',
    },
    { kind: 'code', text: 'cargo run -p migrator -- --dry-run --since 30d' },
    {
      kind: 'attention',
      glyph: '!',
      time: '10:52 PM',
      primary: 'approve',
      secondary: 'reject',
      text: 'migrator wants to run DROP TABLE legacy_txn. 4.2M rows, no reads in 26 hours, snapshot taken at 10:02 PM. This cannot be undone.',
    },
    {
      kind: 'activity',
      lines: ['Task failed in 16m2s: watch v0.2.1 patch release (exit 1)', 'Thought for 5.5s'],
    },
    {
      kind: 'message',
      author: 'agent',
      time: '10:59 PM',
      text: 'That watch process exited after the run had already finished — the migration itself succeeded. Nothing to redo.',
    },
    {
      kind: 'recap',
      text: 'Ledger adapter landed, writes cut over, legacy_txn left readable behind a view. One approval is still waiting: dropping the old table.',
    },
    { kind: 'live', text: 'verifier · reconciling 4.2M rows · 2m 14s' },
  ],
  onboarding: [
    {
      kind: 'message',
      author: 'user',
      time: '08:05 AM',
      text: 'Rewrite the onboarding flow to three steps and drop the tour modal.',
    },
    {
      kind: 'activity',
      lines: ['Spawned 2 agents · planner, editor', 'Read 14 files', 'Thought for 2.1s'],
    },
    {
      kind: 'attention',
      glyph: '✓',
      time: '08:11 AM',
      primary: 'approve plan',
      secondary: 'edit plan',
      text: 'Nine steps across 14 files, ending with the tour modal deleted. Nothing runs until you sign off.',
    },
  ],
  support: [
    {
      kind: 'message',
      author: 'user',
      time: '06:30 AM',
      text: 'Triage the overnight support queue and draft replies for anything about failed payouts.',
    },
    {
      kind: 'activity',
      lines: ['Spawned 1 agent · triager', 'Read 41 tickets', 'Thought for 0.7s'],
    },
    {
      kind: 'message',
      author: 'agent',
      time: '07:04 AM',
      text: 'Drafted 18 replies and escalated 4. Stopped at the spend cap with 9 tickets left.',
    },
    {
      kind: 'recap',
      text: 'Payout failures were all the same expired-webhook cause; the escalations are the four with disputed amounts.',
    },
  ],
  docs: [],
};

export function projectById(projects: readonly Project[], id: string): Project {
  const first = projects[0];
  if (!first) throw new Error('The workspace needs at least one project.');
  return projects.find((project) => project.id === id) ?? first;
}

export function appendTask(
  entries: readonly ThreadEntry[],
  draft: string,
  now: Date,
): readonly ThreadEntry[] {
  const text = draft.trim();
  if (!text) return entries;

  const time = now.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
  return [
    ...entries,
    { kind: 'message', author: 'user', text, time },
    { kind: 'activity', lines: ['Spawned 2 agents · planner, worker', 'Reading context'] },
    { kind: 'live', text: 'planner · drafting a plan · 3s' },
  ];
}

export function resolveAttention(
  entries: readonly ThreadEntry[],
  index: number,
  resolution: string,
): readonly ThreadEntry[] {
  if (entries[index]?.kind !== 'attention') return entries;
  return entries.map((entry, entryIndex) =>
    entryIndex === index ? { kind: 'activity', lines: [resolution] } : entry,
  );
}
