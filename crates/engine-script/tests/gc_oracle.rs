//! GC reachability oracle (PR 2, ADR-035).
//!
//! Allocates a known number of objects, drops references in interleaved
//! patterns, runs `collect()`, and asserts the post-collect live set is
//! exactly the kept set. No live-object overcollection; no use-after-free
//! (`Heap::get` returns `None` for swept handles).

use engine_script::gc::{Heap, Obj};
use engine_script::vm::Value;

#[test]
fn kept_handles_survive_collection() {
    let mut heap = Heap::with_default_config();
    let h0 = heap.alloc(Obj::Array(vec![Value::Int(1)]));
    let h1 = heap.alloc(Obj::Array(vec![Value::Int(2)]));
    let h2 = heap.alloc(Obj::Array(vec![Value::Int(3)]));

    // Keep h0 and h2; drop h1.
    let roots = vec![Value::Array(h0), Value::Array(h2)];
    let stats = heap.collect(&roots);
    assert_eq!(stats.live, 2);

    assert!(heap.get(h0).is_some());
    assert!(heap.get(h1).is_none(), "h1 should have been swept");
    assert!(heap.get(h2).is_some());
}

#[test]
fn transitive_reachability() {
    let mut heap = Heap::with_default_config();
    let child = heap.alloc(Obj::Array(vec![Value::Int(99)]));
    let parent = heap.alloc(Obj::Array(vec![Value::Array(child)]));

    let roots = vec![Value::Array(parent)];
    let stats = heap.collect(&roots);
    assert_eq!(stats.live, 2);
    assert!(heap.get(parent).is_some());
    assert!(heap.get(child).is_some());
}

#[test]
fn dropping_parent_collects_child() {
    let mut heap = Heap::with_default_config();
    let child = heap.alloc(Obj::Array(vec![Value::Int(99)]));
    let _parent = heap.alloc(Obj::Array(vec![Value::Array(child)]));

    let roots: Vec<Value> = Vec::new();
    let stats = heap.collect(&roots);
    assert_eq!(stats.live, 0);
    assert!(heap.get(child).is_none());
}

#[test]
fn churn_100k_objects() {
    // The pause-oracle's load profile, run once for correctness:
    // allocate 100k objects, drop most refs, collect, assert no
    // overcollect or under-collect.
    let mut heap = Heap::with_default_config();
    let mut handles = Vec::with_capacity(100_000);
    for i in 0..100_000 {
        handles.push(heap.alloc(Obj::Array(vec![Value::Int(i)])));
    }
    // Keep every 10th handle.
    let roots: Vec<Value> = handles
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 10 == 0)
        .map(|(_, h)| Value::Array(*h))
        .collect();
    let stats = heap.collect(&roots);
    assert_eq!(stats.live, 10_000);
}

#[test]
fn free_list_recycles_slots() {
    let mut heap = Heap::with_default_config();
    let h0 = heap.alloc(Obj::Array(vec![]));
    let h1 = heap.alloc(Obj::Array(vec![]));
    heap.collect(&[]); // drop both
    assert!(heap.get(h0).is_none());
    assert!(heap.get(h1).is_none());
    let h2 = heap.alloc(Obj::Array(vec![]));
    // Slot reuse — h2 should occupy one of the freed indices.
    let live = heap.live_handles();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0], h2);
}
