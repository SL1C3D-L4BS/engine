//! Render-graph topological-sort determinism oracle (ADR-039 §Verification).
//!
//! Registers a synthetic 6-pass graph with declared dependencies and
//! asserts a stable execution order regardless of how many times the
//! graph is compiled, and regardless of whether independent passes
//! were registered in different orders within their tied tier.
//!
//! The mirror-image of the replay-parity oracle in `engine-core`
//! (ADR-033): the same graph topology + same registration order →
//! byte-identical scheduling decision.

use engine_render::render_graph::{
    Pass, PassContext, RenderGraph, ResourceId, ResourceSet, Track,
};

/// A trivial pass that declares a fixed read/write resource set.
struct P {
    name: &'static str,
    reads: Vec<ResourceId>,
    writes: Vec<ResourceId>,
}

impl Pass for P {
    fn name(&self) -> &'static str {
        self.name
    }
    fn track(&self) -> Track {
        Track::A
    }
    fn reads(&self, s: &mut ResourceSet) {
        for r in &self.reads {
            s.add(*r);
        }
    }
    fn writes(&self, s: &mut ResourceSet) {
        for r in &self.writes {
            s.add(*r);
        }
    }
    fn record(&mut self, _: &mut PassContext) {}
}

fn make_graph() -> RenderGraph {
    // Resource ids per ADR-053 PR 1 reference table:
    //   r0 = SceneVisibility (geom.feed writes; cull reads)
    //   r1 = CulledQueue     (cull writes; depth+draw read)
    //   r2 = Depth           (depth_prepass writes; draw_opaque reads)
    //   r3 = GBuffer         (draw_opaque writes; shade reads)
    //   r4 = ShadingResult   (shade writes; tonemap reads)
    //   r5 = SwapchainImage  (tonemap writes)
    let r0 = ResourceId(0);
    let r1 = ResourceId(1);
    let r2 = ResourceId(2);
    let r3 = ResourceId(3);
    let r4 = ResourceId(4);
    let r5 = ResourceId(5);

    let mut g = RenderGraph::new();
    g.add_pass(P {
        name: "geom.feed",
        reads: vec![],
        writes: vec![r0],
    });
    g.add_pass(P {
        name: "cull",
        reads: vec![r0],
        writes: vec![r1],
    });
    g.add_pass(P {
        name: "depth_prepass",
        reads: vec![r1],
        writes: vec![r2],
    });
    g.add_pass(P {
        name: "draw_opaque",
        reads: vec![r1, r2],
        writes: vec![r3],
    });
    g.add_pass(P {
        name: "shade",
        reads: vec![r3],
        writes: vec![r4],
    });
    g.add_pass(P {
        name: "tonemap",
        reads: vec![r4],
        writes: vec![r5],
    });
    g
}

#[test]
fn topo_order_is_stable_across_recompiles() {
    let mut g = make_graph();
    g.compile().unwrap();
    let first = g.scheduled_names().unwrap();
    // Recompile a few times; identical schedule each time.
    for _ in 0..5 {
        g.compile().unwrap();
        assert_eq!(g.scheduled_names().unwrap(), first);
    }
}

#[test]
fn topo_order_matches_data_dependencies() {
    let mut g = make_graph();
    g.compile().unwrap();
    let order = g.scheduled_names().unwrap();
    // Position checks: each pass must come after every dependency.
    fn pos(order: &[&'static str], name: &str) -> usize {
        order.iter().position(|n| *n == name).unwrap()
    }
    assert!(pos(&order, "geom.feed") < pos(&order, "cull"));
    assert!(pos(&order, "cull") < pos(&order, "depth_prepass"));
    assert!(pos(&order, "cull") < pos(&order, "draw_opaque"));
    assert!(pos(&order, "depth_prepass") < pos(&order, "draw_opaque"));
    assert!(pos(&order, "draw_opaque") < pos(&order, "shade"));
    assert!(pos(&order, "shade") < pos(&order, "tonemap"));
}

#[test]
fn independent_passes_emit_in_registration_order() {
    // No data dependencies between A and B; registration order
    // pins their relative order.
    let mut g1 = RenderGraph::new();
    g1.add_pass(P {
        name: "indep_a",
        reads: vec![],
        writes: vec![ResourceId(10)],
    });
    g1.add_pass(P {
        name: "indep_b",
        reads: vec![],
        writes: vec![ResourceId(11)],
    });
    g1.compile().unwrap();
    assert_eq!(g1.scheduled_names().unwrap(), vec!["indep_a", "indep_b"]);

    let mut g2 = RenderGraph::new();
    g2.add_pass(P {
        name: "indep_b",
        reads: vec![],
        writes: vec![ResourceId(11)],
    });
    g2.add_pass(P {
        name: "indep_a",
        reads: vec![],
        writes: vec![ResourceId(10)],
    });
    g2.compile().unwrap();
    // Reversed registration → reversed schedule. Determinism is per
    // registration order, not per pass name.
    assert_eq!(g2.scheduled_names().unwrap(), vec!["indep_b", "indep_a"]);
}

#[test]
fn track_b_alternative_replaces_named_track_a_pass() {
    // A Track::B pass declares it replaces "cull"; with the runtime
    // switch on Track::B, the graph schedules it instead of "cull".
    struct GpuCull;
    impl Pass for GpuCull {
        fn name(&self) -> &'static str {
            "gpu_driven_cull"
        }
        fn track(&self) -> Track {
            Track::B
        }
        fn replaces(&self) -> &'static [&'static str] {
            &["cull"]
        }
        fn reads(&self, s: &mut ResourceSet) {
            s.add(ResourceId(0));
        }
        fn writes(&self, s: &mut ResourceSet) {
            s.add(ResourceId(1));
        }
        fn record(&mut self, _: &mut PassContext) {}
    }

    let mut g = make_graph();
    g.add_pass(GpuCull);

    // Track::A — gpu_driven_cull excluded.
    g.compile().unwrap();
    let names_a = g.scheduled_names().unwrap();
    assert!(names_a.contains(&"cull"));
    assert!(!names_a.contains(&"gpu_driven_cull"));

    // Track::B — only the B pass plus the Both passes (none here).
    g.set_track(Track::B);
    g.compile().unwrap();
    let names_b = g.scheduled_names().unwrap();
    assert!(!names_b.contains(&"cull"));
    assert!(names_b.contains(&"gpu_driven_cull"));
}
