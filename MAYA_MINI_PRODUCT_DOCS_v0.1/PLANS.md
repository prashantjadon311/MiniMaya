# Maya Mini — Implementation Plan Protocol

Use an **ExecPlan** only when an approved task spans >1 module, changes an API/storage schema/security boundary, or cannot be reviewed as a small isolated patch.

Every plan MUST contain:
1. **Scope:** approved milestone + work package IDs, corresponding feature IDs, explicit exclusions.
2. **Context:** files/docs examined and verified current repo status.
3. **Design:** components/interface and why this is the smallest approach.
4. **Safety:** paths, secrets, privileged commands, browser terms, rollback, failure handling.
5. **Steps:** smallest incremental implementation/testing checkpoints with dependencies.
6. **Tests:** test IDs and commands; add negative/security and resource checks as relevant.
7. **Evidence:** exact observable acceptance behavior, not a vague 'working' claim.
8. **Approval:** sign-off before product-code changes, not merely before planning.
9. **Handoff:** changed files, what passed/failed/not run, outstanding risks, continuation point.

Work plans live under `workplans/WP-<id>-<slug>.md` **only after that work package is approved**. Do not generate dozens of speculative plan files. Updates to this document itself do not authorize implementation.
