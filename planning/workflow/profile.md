# Workflow Repository Profile

The workflow profile at `.agents/research-workflow.json` configures repository-specific paths, git environments, and concurrency limits while keeping the underlying tooling and skills portable.

## Schema & Keys

```json
{
  "version": 1,
  "workflow_docs": "planning/workflow",
  "requests_root": "doc",
  "plans_root": "planning/plans",
  "reference_roots": ["doc/references", "planning"],
  "request_catalog": "doc/README.md",
  "plan_catalog": "planning/plans/README.md",
  "issue_repository": "developing-today/code",
  "trunk": "main",
  "integration_prefix": "integration/",
  "worktree_root": "/home/user/work/wt",
  "git_env": "source /home/user/.local/share/plaza-tools/git-env.sh",
  "max_parallel_writers": 5,
  "merge_method": "merge",
  "validation_commands": ["python3 scripts/research_workflow.py check --repo-root ."],
  "lifecycle_skills": ["research-workflow", "request-intake", "research-prepare", "concept-research", "notes-reconcile", "issue-handoff", "design-orchestrate", "feature-plan", "implementation-orchestrate"],
  "rulebook_budget": {
    "AGENTS.md": 8000,
    "CLAUDE.md": 8000
  }
}
```

### Key Reference

| Key                    | Type          | Description                                                                                                                    |
| ---------------------- | ------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `version`              | Integer       | Configuration schema version (must be `1`).                                                                                    |
| `workflow_docs`        | String        | Relative path to the workflow documentation library.                                                                           |
| `requests_root`        | String        | Directory where research packages are scaffolded (`doc`).                                                                      |
| `plans_root`           | String        | Directory where implementation plans live (`planning/plans`).                                                                  |
| `reference_roots`      | Array[String] | Directories containing valid targets for documentation links.                                                                  |
| `request_catalog`      | String        | Path to the catalog file indexing research requests.                                                                           |
| `plan_catalog`         | String        | Path to the catalog file indexing implementation plans.                                                                        |
| `issue_repository`     | String        | GitHub repository identifier (`owner/repo`) for issue creation.                                                                |
| `trunk`                | String        | Default base branch for pull requests (typically `main`).                                                                      |
| `integration_prefix`   | String        | Prefix used for multi-stage integration branches (`integration/`).                                                             |
| `worktree_root`        | String        | Directory where parallel git worktrees are instantiated.                                                                       |
| `git_env`              | String        | Shell command required to initialize git environment before operations.                                                        |
| `agent_work_dir`       | String        | Optional. Live coordination directory (locks, heartbeats) when `$(git rev-parse --git-common-dir)/agent-work` is not writable. |
| `max_parallel_writers` | Integer       | Hard limit on concurrent active subagent worktrees.                                                                            |
| `merge_method`         | String        | Git merge strategy (must be `merge` to preserve merge commits).                                                                |
| `validation_commands`  | Array[String] | Verification commands executed during CI checks.                                                                               |
| `lifecycle_skills`     | Array[String] | List of 9 standard skills that must exist in `.agents/skills/`.                                                                |
| `rulebook_budget`      | Object        | Maximum allowed byte sizes for always-loaded agent rulebooks.                                                                  |
