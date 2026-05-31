# 027 — Release pipeline (tag → signed artifacts)

> Cutting a release is one tag away from signed, installable downloads.

**Capability:** [§11 Quality, CI & release readiness](../PRD.md#11-quality-ci--release-readiness) · **Status:** ◻ not started · **Depends on:** 024, 025, 026

## User story
As the owner, I want a tagged release to automatically produce signed, installable artifacts
and the update manifest, so shipping is repeatable and not a manual ritual.

## Scope
- **In:** A release pipeline that, on a version tag, runs the full quality check and produces
  the signed/notarized per-platform artifacts plus the auto-update manifest.
- **Out:** The signing setup itself (024, 025) and update client (026) — this **automates** them.

## Acceptance criteria
- [ ] Pushing a **version tag** triggers a release that **first requires the full CI check to
      pass** (the hard release gate).
- [ ] The release produces **signed/notarized macOS** and **signed Windows** artifacts and the
      **auto-update manifest** that 026 consumes.
- [ ] Released artifacts are **downloadable from a documented location**, with per-platform
      install instructions.
- [ ] A failed gate or signing step **fails the release** without publishing partial artifacts.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
QA: a dry-run/tagged pre-release produces installable artifacts that pass 024–026's checks.

## References
- [QUALITY_GATES.md](../QUALITY_GATES.md) · [OPEN_QUESTIONS D1, D2, O2](../OPEN_QUESTIONS.md)
