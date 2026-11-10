# Issue tracker: Linear

Issues and specs for this repo live in Linear, in the `HVN` (Personal) team, under the `gnome-annotation` project. Use the Linear MCP tools for all operations.

## Conventions

- **Create an issue**: `save_issue` with `team: "HVN"`, `project: "gnome-annotation"`, `title`, and `description`. Descriptions are Markdown.
- **Read an issue**: `get_issue` with the identifier, for example `HVN-12`.
- **List issues**: `list_issues` with `team: "HVN"`, `project: "gnome-annotation"`, and any `label` or `status` filter. Results paginate; pass the cursor while `hasNextPage` is true.
- **Make a sub-issue**: `save_issue` with `parent: "HVN-12"`.
- **Blocking**: Linear has no blocking field in these tools. Put `Blocked by: HVN-12, HVN-15` as the first line of the description. A ticket is unblocked when every blocker is in `Done`.
- **Comment**: `save_comment` with `issueId` and `body`.
- **Apply or remove labels**: `save_issue` with `addLabels` or `removeLabels`. The `labels` field replaces the full set.
- **Close**: set `status: "Done"` with `save_issue`. For `wontfix`, post the explanation first, then set `status: "Canceled"`.
- **Statuses**: `Backlog`, `Todo`, `In Progress`, `In Review`, `Done`, `Canceled`, `Duplicate`.

## Scope

Team key `HVN`, project `gnome-annotation`. Do not file into another team or project unless the user asks.

## Rejected requests

`/triage` records rejected enhancements as files under `.out-of-scope/` at the repo root and links them from the closing comment.

## When a skill says "publish to the issue tracker"

Create a Linear issue in `HVN` under the `gnome-annotation` project.

## When a skill says "fetch the relevant ticket"

Read it with `get_issue` using the `HVN-` identifier.

## Wayfinding operations

Used by `/wayfinder`. The map is a single issue labelled `wayfinder:map`; child tickets are its sub-issues.

- **Map**: one issue labelled `wayfinder:map`, holding the Notes, Decisions-so-far, and Fog body.
- **Child ticket**: a sub-issue of the map. Labels: `wayfinder:research`, `wayfinder:prototype`, `wayfinder:grilling`, or `wayfinder:task`.
- **Blocking**: a `Blocked by:` line in the child description.
- **Frontier query**: list the map's open sub-issues, drop any with an open blocker or an assignee. First in map order wins.
- **Claim**: `save_issue` with `assignee: "me"`. This is the session's first write.
- **Resolve**: post the answer with `save_comment`, set `status: "Done"`, then append a context pointer to the map's Decisions-so-far.
