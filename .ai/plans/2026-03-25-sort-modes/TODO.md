# TODO

## Implementation
- [ ] Add `indexmap = "2"` to Cargo.toml
- [ ] Define `SortMode` enum in `state.rs`
- [ ] Add `sort_mode` field to `AppState`
- [ ] Add `CycleSortMode` to `Action` enum
- [ ] Implement `sort_agents()` and `status_priority()` in `state.rs`
- [ ] Replace BTreeMap -> IndexMap in `agent_tree.rs`
- [ ] Replace BTreeMap -> IndexMap in `build_nav_items()`
- [ ] Wire `s` keybind to CycleSortMode, `S` to ToggleSubagentLog
- [ ] Handle CycleSortMode action in run_loop
- [ ] Apply sort after MonitorUpdate with cursor preservation
- [ ] Add sort mode indicator to footer
- [ ] Update help text

## Tests
- [ ] `test_sort_mode_cycling` - Activity -> Status -> Activity
- [ ] `test_sort_mode_label` - verify label strings
- [ ] `test_cycle_sort_mode_on_state` - state mutation test
- [ ] `test_sort_by_status_ordering` - Processing first, then Idle
- [ ] `test_sort_by_activity_ordering` - most recent first
- [ ] `test_build_nav_items_preserves_sort_order` - IndexMap preserves order
- [ ] `test_sidebar_s_cycles_sort` - keybind mapping
- [ ] `test_sidebar_shift_s_toggles_subagent_log` - keybind mapping

## Validation
- [ ] `cargo test` passes
- [ ] `cargo clippy` clean
- [ ] Manual test: launch with multiple sessions, verify sort toggles work
