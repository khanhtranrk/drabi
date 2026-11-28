# Contributing

All contributions, including documentation and AI-assisted changes, must follow an issue-first, pull-request-based workflow. Use English for issues, pull requests, commit messages, and repository documentation.

## Workflow

1. Read [README.md](README.md) and [AGENTS.md](AGENTS.md). Search existing issues and pull requests.
2. Reuse a relevant issue or create one with the problem, scope, and acceptance criteria. Seek maintainer agreement before significant design changes.
3. Create a dedicated branch from the latest `main`, such as `feat/42-input-routing`, `fix/42-focus`, or `docs/42-guide`.
4. Implement only the agreed scope. Preserve project boundaries and unrelated changes. Update affected tests and documentation.
5. Run relevant checks documented in README.md and AGENTS.md. Report actual results and checks that could not be run.
6. Commit focused changes with descriptive English messages and push only the working branch.
7. Open a pull request targeting `main`. Link the issue using `Closes #42` when fully resolved, summarize changes, validation, and limitations. Use a draft PR for incomplete work.
8. Address feedback on the same branch. Leave approval and merging to the maintainer.

## Repository safety

- Never commit or push directly to `main`, even for small fixes or documentation.
- Never force-push, rewrite shared history, bypass checks, or merge your own PR.
- Do not publish releases, push release tags, or change repository settings without explicit maintainer authorization.
- Never include secrets or unrelated local changes.
- Report access or validation blockers instead of bypassing the workflow.

## AI-assisted contributions

Agents must follow this guide and AGENTS.md. An implementation request does not authorize merging or releasing. Stop after opening or updating the PR and report the issue, branch, PR, validation results, and remaining limitations.
