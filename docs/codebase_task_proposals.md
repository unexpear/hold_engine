# Codebase task proposals

## 1) Typo fix task
**Title:** Clean up corrupted text in `Stats` component comments.

**Issue found:** `src/ecs/components/stats.rs` contains visibly corrupted comment text: `f32` appears as `32`, and `true` appears as `	rue` in two method docs.

**Task:** Replace the malformed characters with plain ASCII text in the affected doc comments.

**Acceptance criteria:**
- The module-level comment says `f32` correctly.
- Method docs for `spend_mana` and `award_xp` say `Returns true ...`.
- `cargo test` still passes.

---

## 2) Bug fix task
**Title:** Make XP leveling handle multi-level gains in one award.

**Issue found:** `Stats::award_xp` levels up at most once per call, even if `amount` is large enough to cross multiple thresholds.

**Task:** Update `award_xp` so it repeatedly levels while `xp >= xp_next`, and return `true` if at least one level-up occurred.

**Acceptance criteria:**
- Awarding a large XP amount can increase multiple levels in one call.
- The method preserves leftover XP after all level-ups.
- Add tests for single-level and multi-level gain cases.

---

## 3) Comment/documentation discrepancy task
**Title:** Correct stale complexity note in `World::query` docs.

**Issue found:** The `World::query` docs say `filter_map` composition is `O(|A| × log(|B|))`, but this ECS uses sparse-set lookups that are effectively constant-time per entity.

**Task:** Update the complexity note to match current implementation characteristics (e.g., `O(|A|)` with `O(1)` sparse lookups into `B`) and clarify this is still less cache-efficient than purpose-built join queries.

**Acceptance criteria:**
- The complexity statement in `src/ecs/world.rs` is technically accurate.
- The note still guides users toward `query2/query3` for multi-component iteration.

---

## 4) Test improvement task
**Title:** Strengthen scene spawn tests to verify parent linkage correctness.

**Issue found:** `scene::tests::spawn_into_world` only checks that `crate` has a `Parent` component, but not that its parent points to `player`.

**Task:** Extend the test to assert that `Parent::entity` (or equivalent field) on `crate` equals the mapped `player` entity.

**Acceptance criteria:**
- Test fails if a wrong parent is attached.
- Test continues to pass for the current sample scene.
- No behavioral changes to runtime code required.
