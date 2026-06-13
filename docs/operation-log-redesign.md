# Operation Log Redesign

Last updated: June 13, 2026
Status: design proposal (not yet implemented)

## Goal

A single, well-defined list of operations where:

1. **Each operation has a reasonable undo** — and undo/redo work uniformly for
   every kind, including the ones that have no inverse today (recategorize,
   merge, sync, retire).
2. **You can see every operation that happened to a given transaction**,
   following identity changes through **merge** (N→1) and **split** (1→N).

This is the keystone for fixing the architecture deficiencies recorded in the
project memory (`architecture-deficiencies.md`): four overlapping "memory"
mechanisms (git, `operations.jsonl`, resolutions, per-type inverse ops), none
complete or authoritative, plus a `general.journal` that is both a generated
projection and a hand-edited surface.

## The layering: three concerns, two stores, one git floor

| Concern                      | Where it lives                                             | Lifetime                   |
| ---------------------------- | ---------------------------------------------------------- | -------------------------- |
| Recovery / fixing bugs       | **git history** (one commit per op)                        | permanent, never rewritten |
| "What happened" + provenance | **operation log** (semantic records) + a lineage **index** | permanent                  |
| Undo / redo                  | **undo fuel** (per-op snapshots) keyed by `op_id`          | **GC-able**                |

Key decisions already made:

- The **operation log** and the **per-transaction provenance view** are _not two
  logs_ — they are one semantic event stream, indexed two ways (by sequence for
  undo, by lineage for provenance). The log records stay small and permanent.
- **Undo keeps its own snapshots** (decision "B"), stored separately from the
  log and **garbage-collected** past an undo horizon. After GC the operation
  remains visible as provenance but is no longer undoable.
- **git stays separate and below** the app's model — the byte-level escape
  hatch. We do _not_ GC git (that would mean rewriting history) and we do not
  parse it for normal undo/provenance. Every mutation produces exactly one
  commit so git is also a coarse recovery-grade undo.

```
                 ┌──────────────────────────────────────────┐
recovery /       │  git history — byte-level, PERMANENT      │  never rewritten / GC'd
fixing bugs ───► │  one commit per op, tagged with op_id     │
                 └──────────────────────────────────────────┘
                              ▲ writes (same critical section)
                 ┌──────────────────────────────────────────┐
"what happened"  │  operation log — semantic, PERMANENT      │  source of truth for events
(events)         │  op_id, seq, kind, consumed/produced,     │  small records, no snapshots
                 │  source_entries, reversible, git_commit   │
                 └───────────────┬──────────────┬────────────┘
              index by lineage   │              │  window over the tail
                                 ▼              ▼  + pre/post snapshots
                 ┌───────────────────┐  ┌───────────────────────────┐
provenance ────► │ per-txn lineage   │  │ undo fuel — GC-able        │ ◄── undo / redo
(PERMANENT)      │ index (derived)   │  │ bookkeeping/undo/<op_id>   │
                 └───────────────────┘  └───────────────────────────┘
```

## On-disk layout

```text
operations.jsonl                         # the operation log (one JSON record per line)
bookkeeping/operations-index.jsonl       # derived lineage/seq index (rebuildable)
bookkeeping/undo/<op_id>.json            # undo fuel: pre/post snapshots (GC-able)
```

The log and the undo fuel are themselves tracked in git, so recovery includes
them. The index is rebuildable from the log and need not be committed.

(Replaces today's split: a write-only GL `operations.jsonl`, a per-account
`operations.jsonl`, and the resolutions store. Resolutions remain — they are
_decisions_, not _operations_ — but operations stop being scattered.)

## The operation record (small, permanent)

```jsonc
{
    "opId": "o-501",
    "seq": 143, // monotonic, ledger-global; the undo ordering
    "ts": "2026-06-13T14:49:00Z",
    "actor": "user", // user | automation | import | migration
    "kind": "merge", // see inventory below
    "consumedTxns": ["7f3a", "9c2e"], // GL txn ids removed/replaced
    "producedTxns": ["b4d1"], // GL txn ids created
    "sourceEntries": ["chase/checking:e12", "amex/card:e88"],
    "reversible": "conditional", // yes | conditional | no
    "commit": "a1b2c3", // the git commit this op produced
}
```

No snapshots here. The bulky pre/post images live in `bookkeeping/undo/<opId>.json`.

## Lineage: per-transaction history through merge & split

Each operation is a **hyperedge** connecting the things it touched: source
entries and GL transaction ids, with `consumed → produced` edges. A
transaction's history is **every operation in its connected lineage component**,
ordered by `seq`. This is the only model that survives identity changes:

- `history(b4d1)` after a merge returns: the two posts that created `7f3a` and
  `9c2e`, the merge `{7f3a,9c2e}→b4d1`, and any later recategorize/sync/unpost of
  `b4d1`.
- A split `x → {x1, x2}` chains the same way (one source entry, multiple
  produced txns).

Indexing operations by a single txn id cannot do this; lineage edges can. The
index also keys on source entry, so `post → unpost → repost` and split legs
chain through the entry even when GL ids churn.

## Undo / redo

- **Exposure:** undo is targeted from a transaction's history view ("undo this
  operation"), plus a global "undo last" over `seq`. Redo re-applies (standard
  editor semantics: a new forward op clears the redo stack).
- **Mechanism:** apply the operation's stored inverse from
  `bookkeeping/undo/<opId>.json`. Most ops need only a tiny semantic diff
  (recategorize = old account); the few that rewrote a whole block (sync, merge)
  store the full prior block(s). Redo uses the post-image (or re-runs the
  forward op deterministically).
- **Lineage safety:** an op `O` can be undone only when nothing later (and not
  itself already undone) has consumed `O`'s outputs. Otherwise undo the
  dependents first, or block with an explanation. This makes `reversible`
  load-bearing instead of advisory.

## Garbage collection / undo horizon

Undo fuel is pruned; the log record and the git commit are not. **Recommended
default (confirm):**

- **Hard floor — accounting boundary:** retain an op's fuel until its
  transactions are included in a **finalized reconciliation session** or fall
  within a **soft-closed period**. Once the books are committed past that point,
  you can no longer undo through it. This matches the accounting mental model.
- **Safety cap:** _also_ always retain at least the last **N ops** (default 200)
  or **T days** (default 30), whichever is larger, so a fresh unreconciled
  ledger keeps a generous undo window and fuel can't grow unbounded.
- **When GC runs:** opportunistically on ledger open and after finalize/close.

Alternatives considered: pure count/time cap (simpler, ignores accounting
state). The accounting-boundary floor is preferred because "I closed the month"
is exactly when a user stops wanting to undo into that month.

## Consistency rules

- One operation = one critical section that writes (a) the journal mutation,
  (b) the operation-log record, (c) the undo fuel, and (d) **one git commit**
  covering all touched files. Today posts commit only journals; unpost / sync /
  retire / overlay edits don't commit at all — every op must commit here.
- **Commit failure is a hard error**, not `eprintln!` — disk and HEAD must not
  silently diverge (see the data-loss findings in `architecture-deficiencies.md`).
- The op record stores its `commit` hash so the log and git stay correlated.

## Mutation inventory (what becomes an operation)

GL-identity Δ: `∅→x` create, `x→∅` destroy, `x→x` edit-in-place, `{a,b}→c` merge.

| Operation                   | GL identity Δ       | Inverse             | Snapshot needed for undo                        |
| --------------------------- | ------------------- | ------------------- | ----------------------------------------------- |
| post / post-split / per-leg | `∅→x`               | unpost              | none (delete produced block)                    |
| link transfer               | `∅→x` (2 sources)   | unpost              | none                                            |
| unpost                      | `x→∅`               | re-post             | removed block + prior `posted` refs             |
| sync                        | `x→x`               | restore prior block | full prior block                                |
| recategorize                | `x→x`               | recategorize back   | old account (tiny diff)                         |
| merge                       | `{a,b}→c`           | split back into a,b | both prior blocks + prior `posted`/overlay refs |
| retire pending              | entry delete        | re-insert entry     | full removed source entry                       |
| manual add                  | `∅→x`               | delete block        | none                                            |
| dedup create/update         | entry create/update | (complex)           | prior entry state                               |
| import-dup-repair           | `x→∅`               | re-add              | removed entry + block                           |
| reconcile / link / close    | overlay JSON        | delete/restore JSON | prior JSON object                               |

Today's gaps this closes: **merge, recategorize, manual-add, and overlay edits
log nothing**, and **unpost / sync / retire / overlay don't commit**.

## Rollout

- **Forward-only.** New operations are logged + indexed going forward; no
  backfill of historical commits (old commits lack `op_id` trailers/records).
- **Old transactions** fall back to raw `git log -- <file>` for provenance until
  they are next touched by a logged op.
- **Retire** the two legacy `operations.jsonl` writers once the new log is the
  single path; keep them dual-writing only during the transition if needed.

## Implementation sequence

1. **Stop the bleeding** (independent of this design): add the missing
   guards/commits for the data-loss bugs in `architecture-deficiencies.md`
   (posted-entry re-sync, split-sync clobber, unpost-deletes-reconciled,
   per-op commit). These make every mutation commit — a prerequisite anyway.
2. **Operation record + log writer** behind every mutation (replace the two ad-hoc
   `GlOperation`/`AccountOperation` enums with the unified record).
3. **Undo fuel store** + per-op inverse implementations (start with the ones
   that have no inverse today: recategorize, merge, sync, retire).
4. **Lineage index + per-txn history view** in the UI.
5. **Global undo/redo** over `seq`, with lineage safety.
6. **GC** with the undo horizon.

## Related, separate

- Caching the categorizer model (memoize `MnbModel` on the GL file hash) is a
  categorize-perf follow-up, not part of this redesign. See the post-fix note
  after commit `ab62f4c`.
