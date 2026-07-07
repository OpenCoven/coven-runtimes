# Approve-flow test (throwaway)

This file exists only to create a PR that touches a CODEOWNERS-protected path
(`.github/**`), so we can verify that a code-owner **Approve** from `@romgenie`
registers on GitHub and satisfies branch protection.

Expected: once romgenie submits an Approve, the PR's `reviewDecision` becomes
`APPROVED` and it turns mergeable. This PR is safe to close without merging.
