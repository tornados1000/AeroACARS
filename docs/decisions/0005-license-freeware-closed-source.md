# ADR-0005: License model — freeware, closed-source

- **Status:** Accepted
- **Date:** 2026-05-01
- **Deciders:** Project owner

## Context

CloudeAcars is built for the virtual-aviation community (pilots and virtual airlines on phpVMS). Two questions need answering up front:

1. Does it cost money to use?
2. Is the source code public?

## Decision

- **Cost to end user:** **Free of charge.** No subscription, no per-VA fee, no per-pilot fee.
- **Source code:** **Closed-source.** Source is *not* publicly distributed.

## Rationale (project owner statement)

> *"Lizenz soll nix kosten aber der code auch nicht quell offen sein"* (kickoff, 2026-05-01)

## Implementation guidance

- **Repository:** Private from day one (Git remote is private — GitHub/GitLab/self-hosted; provider TBD).
- **Distribution:** Only built, code-signed binaries are released. No public source artifacts (no public tarballs, no `git archive`, no source maps in releases).
- **EULA:** Phase 5 task. Outline:
  - Free to use for personal and VA-internal use.
  - No redistribution of source.
  - No reverse engineering, decompilation, or extraction beyond what local law forces.
  - No modification.
  - No warranty.
  - Liability cap (typical freeware boilerplate).
- **Third-party libraries:** All bundled OSS dependencies must be audited for license compatibility with a closed-source release product (MIT, Apache-2.0, BSD = fine; GPL/AGPL = excluded; LGPL = case-by-case with dynamic linking only). Tracked in `client/THIRD_PARTY_NOTICES.md` (Phase 5 deliverable).
- **For now:** A minimal `LICENSE` file at repo root carries a placeholder ("All rights reserved. Free of charge for end users. Full EULA pending.").

## Consequences

- **Positive:** Lowest possible adoption barrier for end users.
- **Positive:** We retain full control over the codebase, branding, and feature direction.
- **Negative:** No community contributions via PRs. We absorb 100% of maintenance.
- **Negative:** No revenue — operating costs (code-signing certs, Apple Developer account, optional cloud crash-reporting) come out of the project owner's pocket. This is a Phase-5 budgeting concern.
