# ETHPayServer vX.Y.Z[-alpha]

<!--
  Copy to RELEASE_NOTES_vX.Y.Z.md, fill in, then:  scripts/release.sh vX.Y.Z

  Two rules, because they are what actually made the v0.1.0-alpha notes worth
  reading, and neither is enforceable by structure alone:

  1. LEAD WITH THE CLAIM ONLY THIS RELEASE CAN MAKE. Not a feature list. For
     v0.1.0-alpha it was "the money path is verified end to end, nightly" —
     the one thing no previous state of the project could say. Work out what
     that is for this release before writing anything else. If there isn't
     one, ask whether this needs a release.

  2. NAME GAPS WITH TICKET NUMBERS. "Recovery has no UI (RCS-205)" is
     checkable. "Some features are incomplete" is noise a reader learns to
     skip, and the first person who hits the gap themselves will stop
     trusting everything else on the page.

  No marketing adjectives. For a payment processor the credibility comes from
  being the project that tells you what is broken — anyone can claim fast and
  secure, and nobody believes it.

  Scale by release type. Sections marked REQUIRED are required on every
  release including alphas and patches; the rest are dropped when empty.
-->

**One line: what this is and who it is for.**
State plainly whether it is a pre-release and what that means here.

---

## Highlights
<!-- Minor and stable releases. Patches: skip, the change list is enough. -->

The claim only this release can make goes first, with the evidence for it.
Then what else changed that a user would notice.

## Changes
<!-- REQUIRED. Patch releases: a terse list is fine. Minor/stable: group them. -->

- Grouped by area for minor and stable releases; a flat list for patches.
- Reference tickets and PRs so a reader can go deeper.

## Security
<!-- REQUIRED on stable. Alphas and patches: include if anything applies. -->

Anything fixed that affected authentication, key handling, funds, or data
exposure. Say what was possible before the fix, not just that something was
hardened — a reader needs to judge whether they were exposed.

## Not ready for
<!--
  REQUIRED ON EVERY RELEASE. This is the section that earns the template.

  It is also the one that gets skipped when a release feels routine or you are
  tired, which is exactly when it matters. scripts/release.sh refuses to tag if
  this section is missing or still holds placeholder text.

  On a stable release this should be short. If it is long, the release is
  probably not stable.
-->

**Do not put real funds through this.** ← drop on stable, keep on every alpha.

- Known gap, with the ticket number that tracks it.
- What a user should not attempt yet, and why.

## Upgrade notes
<!-- Include when there are migrations, config changes, or breaking changes. -->

Anything an operator must do by hand. Call out migrations that hold locks, and
anything that is not backward compatible with the previous binary during a
rolling deploy.

## Requirements
<!-- REQUIRED on stable. Otherwise link to README. -->

Runtime and service versions, and where to find setup instructions.

## Feedback

Where to report problems, and what report would be most useful.
