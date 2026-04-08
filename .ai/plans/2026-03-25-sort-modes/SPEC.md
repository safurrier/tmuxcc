# Sort Modes for tmuxcc

## Problem Statement
The sidebar tree view always displays sessions alphabetically (BTreeMap), ignoring the upstream sort from `sort_agents_by_activity()`. Users want to sort by recency and agent status.

## Requirements
1. **Sort by recent activity** (default) - sessions/agents ordered by most recent `last_updated`
2. **Sort by agent status** - running first, then recently finished, then idle
3. **Keybind `s`** cycles sort modes, shown in footer
4. Sort order must be preserved through the rendering pipeline (currently broken by BTreeMap)

## Constraints
- Cursor must remain on the same agent when sort order changes
- Minimal new dependencies (indexmap only)
- Tests for sort logic, nav item ordering, and keybinding mapping
