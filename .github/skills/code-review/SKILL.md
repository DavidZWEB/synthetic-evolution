---
name: code-review
description: Independently review a pull request, branch, staged diff, or unstaged diff against Synthetic Evolution's repository rules. Use after opening or updating a pull request and whenever asked for a code review. The review must be performed by a separate code-review agent running Claude Opus 5 with maximum reasoning.
---

# Independent code review

A review must be independent from the agent that implemented the change. The parent
agent coordinates the review and handles any follow-up; it does not substitute its own
assessment for the independent reviewer's pass.

## Required reviewer

Invoke the task tool with all of these settings:

- `agent_type: code-review`
- `model: claude-opus-5`
- `reasoning_effort: max`
- `context_tier: long_context`
- `mode: sync`, unless there is genuine independent work to do while it runs

Do not silently use another model, lower the reasoning effort, or perform the review
yourself. If that reviewer is unavailable, report the review as blocked rather than
calling a weaker pass complete.

The review agent is read-only. Do not ask it to edit files, commit, push, resolve
threads, or approve or merge a pull request.

## Establish the review scope

Before launching the reviewer:

1. Identify the repository root, current worktree, current branch, working-tree state,
   and the exact comparison base. For a pull request, include its number and base
   branch. For local work, say explicitly whether the scope is staged changes,
   unstaged changes, all local changes, or the branch diff.
2. Include the user's intended behavior and acceptance criteria. State facts, not the
   implementing agent's conclusions about correctness.
3. Tell the reviewer to inspect the actual diff and any callers, tests, generated
   contracts, or documentation needed to understand it. Do not paste a large diff into
   the prompt when the reviewer can read it directly.
4. Require the reviewer to read `AGENTS.md` first. For changes to `sim-core`,
   simulation behavior, or serialized world state, require the spec reading mandated
   there. For other surfaces, point it only to the relevant documentation.

If there is no concrete change set, stop and say that a code review needs a pull
request, branch diff, staged diff, or unstaged diff.

## Reviewer prompt

Give the independent agent a complete prompt containing:

- the worktree path and precise diff scope;
- the original user request and any deliberate behavior changes;
- an instruction to apply `AGENTS.md`, including its invariants, load-bearing rules,
  design guidance, validation requirements, and landing rules;
- an instruction to trace changed callers and data contracts rather than reviewing
  changed lines in isolation;
- an instruction to check whether each changed or added test would fail for the bug it
  claims to prevent;
- an instruction to report only high-confidence correctness, security, determinism,
  compatibility, performance, and reliability defects;
- an instruction to omit style preferences, speculative concerns, and unrelated
  pre-existing problems;
- an instruction to return findings with severity, exact file and line references,
  impact, triggering conditions, and concise fix direction;
- an instruction to state explicitly when the review is clean.

Ask the reviewer to investigate the repository itself. Do not bias it with a proposed
finding or tell it that the implementation is probably correct.

## Handling findings

Verify that every reported location still matches the reviewed revision before acting.
Do not dismiss a finding merely because automated checks pass.

- If the user asked only for a review, report findings without modifying the branch.
- During implementation work, fix clear defects that are within scope. Follow
  `AGENTS.md` for commits made after a pull request opens; do not amend them into the
  original commit.
- Treat design or product judgment as a question for the human rather than silently
  changing behavior.
- After corrections, send the same reviewer a follow-up with `write_agent` and ask it
  to review the correction against its original finding and the updated full diff.
  Keep the reviewer separate from the implementation.
- Continue until the independent reviewer reports no remaining high-confidence
  findings or a human decision is required.

Do not post GitHub review comments, approve the pull request, or merge it unless the
user requested that action.

## Reporting

Lead with the findings, ordered by severity. For each finding, include:

- severity and a short title;
- `path:line`;
- the concrete failure mode and who or what it affects;
- the conditions that trigger it;
- the smallest credible fix direction.

Keep unresolved questions separate from findings. If there are no findings, say the
independent review is clean. Name any scope that could not be reviewed rather than
implying complete coverage.
