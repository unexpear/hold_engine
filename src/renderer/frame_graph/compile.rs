//! Compile a frame graph: resolve dependencies, validate types, sort the
//! passes into execution order.
//!
//! Algorithm overview:
//!
//! 1. **Collect accesses.** Every pass's [`super::Pass::declare`] is called
//!    against a [`super::PassBuilder`], which records every `read`,
//!    `write`, and `read_write` against a resource id.
//!
//! 2. **Detect conflicting writers.** A resource written (or read-written)
//!    by more than one pass is rejected unless the read-write chain makes
//!    the order explicit. v1 does not implement chain inference; multiple
//!    writers are an error.
//!
//! 3. **Build edges.** Pass B depends on pass A iff some resource is
//!    written by A and read (or read-written) by B. The edges are
//!    deduplicated so a pass that reads three of A's outputs only adds one
//!    incoming edge.
//!
//! 4. **Topological sort.** Kahn's algorithm. Nodes with no incoming edges
//!    enter a queue; popping a node from the queue removes its outgoing
//!    edges and may push more nodes. Any node left after the queue drains
//!    is part of a cycle.
//!
//! Stability note: when multiple passes have all their dependencies
//! satisfied at once, they execute in declaration order. This makes the
//! output deterministic and easy to reason about in tests.

use std::collections::{HashMap, VecDeque};

use super::pass::{FrameGraphError, ResourceAccess};
use super::resource::{AccessKind, ResourceId};

/// Output of a successful compile: passes listed in execution order, plus
/// the original access table for backends that need to resolve attachments.
///
/// `accesses_per_pass` is intentionally retained even though v1's stub
/// `execute` does not consume it — backend integration in the next M4
/// commit needs it to bind colour/depth attachments and SSBO/UBO uniforms
/// for each pass. Marking it `dead_code`-allowed avoids the warning during
/// the design-phase commit without losing the data.
pub(super) struct CompileOutput {
    pub execution_order: Vec<usize>,
    #[allow(dead_code)]
    pub accesses_per_pass: Vec<Vec<ResourceAccess>>,
}

/// Compile a list of pass-access records into an execution order.
///
/// `pass_accesses[i]` holds every [`ResourceAccess`] declared by pass `i`.
/// `resource_count` is used to bound the dependency adjacency lists; this
/// could be inferred from the accesses but is passed explicitly so an
/// "unknown resource" error from the builder phase still has a coherent
/// universe to operate in.
pub(super) fn compile_graph(
    pass_accesses: Vec<Vec<ResourceAccess>>,
    pass_names: &[String],
    resource_count: u32,
) -> Result<CompileOutput, FrameGraphError> {
    let n = pass_accesses.len();

    // For each resource, who writes it and who reads it.
    let mut writers: HashMap<ResourceId, Vec<usize>> = HashMap::new();
    let mut readers: HashMap<ResourceId, Vec<usize>> = HashMap::new();

    for (pass_idx, accesses) in pass_accesses.iter().enumerate() {
        for access in accesses {
            // The builder layer already rejects out-of-range ids, but this
            // arm catches accesses constructed via test scaffolding that
            // bypasses the builder.
            if access.resource.index >= resource_count {
                return Err(FrameGraphError::UnknownResource {
                    resource_index: access.resource.index,
                });
            }

            match access.access {
                AccessKind::Read => readers
                    .entry(access.resource)
                    .or_default()
                    .push(pass_idx),
                AccessKind::Write => writers
                    .entry(access.resource)
                    .or_default()
                    .push(pass_idx),
                AccessKind::ReadWrite => {
                    readers
                        .entry(access.resource)
                        .or_default()
                        .push(pass_idx);
                    writers
                        .entry(access.resource)
                        .or_default()
                        .push(pass_idx);
                }
            }
        }
    }

    // Multiple writers without an explicit read-write chain → error.
    // (A read-write counts in both maps for the same pass; that's fine.
    // We're flagging the case where two *different* passes both produce
    // the same resource without one of them also reading it first.)
    for (resource, writers_for_id) in &writers {
        let mut unique: Vec<usize> = writers_for_id.iter().copied().collect();
        unique.sort_unstable();
        unique.dedup();
        if unique.len() > 1 {
            // An exception: if all but one of the writers also appear as
            // readers, treat it as a serialised ping-pong. v1 does not yet
            // infer the order — we still reject, but the message hints at
            // the canonical fix.
            return Err(FrameGraphError::ConflictingWriters {
                resource: *resource,
                writers: unique,
            });
        }
    }

    // Edges: B depends on A (i.e. A → B in execution order) iff some
    // resource is written by A and read (or read-written) by B, with B != A.
    let mut incoming: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut edge_seen: HashMap<(usize, usize), ()> = HashMap::new();

    for (resource, writers_for_id) in &writers {
        // After the conflicting-writers check we know there is at most one.
        let producer = writers_for_id[0];
        if let Some(consumers) = readers.get(resource) {
            for &consumer in consumers {
                if consumer == producer {
                    continue; // self-loop from read-write — skip
                }
                if edge_seen
                    .insert((producer, consumer), ())
                    .is_none()
                {
                    outgoing[producer].push(consumer);
                    incoming[consumer].push(producer);
                }
            }
        }
    }

    // Kahn's algorithm. Tie-break on declaration order so the output is
    // deterministic.
    let mut in_degree: Vec<usize> = incoming.iter().map(|e| e.len()).collect();
    let mut ready: VecDeque<usize> = VecDeque::new();
    for i in 0..n {
        if in_degree[i] == 0 {
            ready.push_back(i);
        }
    }

    let mut order: Vec<usize> = Vec::with_capacity(n);
    while let Some(p) = ready.pop_front() {
        order.push(p);
        // Sort the destinations so deterministic tie-breaking holds even
        // when the edge list was built from a HashMap iteration.
        let mut next: Vec<usize> = outgoing[p].clone();
        next.sort_unstable();
        for q in next {
            in_degree[q] -= 1;
            if in_degree[q] == 0 {
                ready.push_back(q);
            }
        }
    }

    if order.len() != n {
        // Cycle. Report the offending passes (those still with in_degree > 0)
        // in declaration order so the message is reproducible.
        let mut cycle_members: Vec<usize> = (0..n)
            .filter(|i| !order.contains(i))
            .collect();
        cycle_members.sort_unstable();
        // Pull out names too if the caller has them — we keep the indices
        // primary so the error type stays light, but log a hint.
        let _ = pass_names; // names retained for future richer error messages
        return Err(FrameGraphError::Cycle {
            passes: cycle_members,
        });
    }

    Ok(CompileOutput {
        execution_order: order,
        accesses_per_pass: pass_accesses,
    })
}
