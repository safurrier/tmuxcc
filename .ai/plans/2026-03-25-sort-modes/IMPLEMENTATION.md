# Implementation Plan

## Step 1: Add `indexmap` dependency
- `Cargo.toml`: add `indexmap = "2"`

## Step 2: Define `SortMode` enum + state field
- `src/app/state.rs`: Add `SortMode` enum (Activity, Status) with `next()`, `label()`, `Default`
- Add `sort_mode: SortMode` field to `AppState`, init as default
- Add `cycle_sort_mode()` method

## Step 3: Add `CycleSortMode` action
- `src/app/actions.rs`: New variant + description

## Step 4: Implement sort functions
- `src/app/state.rs`: Add `sort_agents()` dispatching on mode
- `SortMode::Activity`: sort sessions by most recent `last_updated` across their agents
- `SortMode::Status`: sort by status_priority (Processing=0, AwaitingApproval=1, Error=2, Unknown=3, Idle=4), then by `last_updated`

## Step 5: Replace BTreeMap with IndexMap
- `src/ui/components/agent_tree.rs`: Change `SessionsMap` and `WindowsMap` from BTreeMap to IndexMap
- `src/app/state.rs` `build_nav_items()`: Change 3 BTreeMap instances to IndexMap
- This preserves insertion order from the sorted `root_agents` vector

## Step 6: Wire keybinding
- `src/ui/app.rs` `map_key_to_action()`: `s` -> CycleSortMode, `S` -> ToggleSubagentLog (was both s/S)

## Step 7: Handle action in run loop
- `src/ui/app.rs`: Handle `Action::CycleSortMode` by calling `state.cycle_sort_mode()`
- After receiving MonitorUpdate, apply `sort_agents(&mut state.agents.root_agents, state.sort_mode)`
- Preserve cursor identity: find current agent target before sort, restore cursor index after

## Step 8: Show sort mode in footer
- `src/ui/components/footer.rs`: Add `s:Recent` / `s:Status` indicator

## Step 9: Update help text
- `src/ui/components/help.rs`: Document `s` = cycle sort, `S` = toggle subagent log

## Step 10: Tests
See TODO.md for full test list.
