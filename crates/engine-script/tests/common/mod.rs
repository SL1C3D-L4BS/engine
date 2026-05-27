//! Shared fixture corpus for the PR-1 oracles.
//!
//! Twenty representative `.sli` programs exercising every front-end
//! surface: arithmetic, control flow, struct declaration / literal /
//! field access, closures, and the ECS query sugar from spec V.3.
//! The corpus is sorted by name; the cross-arch compile_parity golden
//! takes a BLAKE3 digest over the concatenated IR serialisation in
//! that order, so adding a fixture in the middle reorders the digest
//! and is a deliberate change.

/// One named fixture program.
pub struct Fixture {
    pub name: &'static str,
    pub source: &'static str,
}

/// The full corpus, sorted by name.
pub fn corpus() -> Vec<Fixture> {
    let mut v: Vec<Fixture> = vec![
        Fixture {
            name: "01_arith_int",
            source: r#"
fn main() -> i64 {
    return 1 + 2 * 3 - 4 / 2 + 5 % 3;
}
"#,
        },
        Fixture {
            name: "02_arith_float",
            source: r#"
fn main() -> f64 {
    let x: f64 = 3.5;
    let y: f64 = 1.5;
    return x * y - 0.25;
}
"#,
        },
        Fixture {
            name: "03_let_shadowing",
            source: r#"
fn main() -> i64 {
    let x: i64 = 1;
    let x: i64 = x + 2;
    let x: i64 = x * 4;
    return x;
}
"#,
        },
        Fixture {
            name: "04_mut_binding",
            source: r#"
fn main() -> i64 {
    let mut sum: i64 = 0;
    sum = sum + 7;
    sum = sum + 11;
    return sum;
}
"#,
        },
        Fixture {
            name: "05_if_else",
            source: r#"
fn classify(n: i64) -> i64 {
    if n < 0 {
        return -1;
    } else {
        if n == 0 {
            return 0;
        } else {
            return 1;
        }
    }
}
"#,
        },
        Fixture {
            name: "06_while_loop",
            source: r#"
fn sum_to(n: i64) -> i64 {
    let mut i: i64 = 0;
    let mut acc: i64 = 0;
    while i < n {
        acc = acc + i;
        i = i + 1;
    }
    return acc;
}
"#,
        },
        Fixture {
            name: "07_recursive_fib",
            source: r#"
fn fib(n: i64) -> i64 {
    if n < 2 {
        return n;
    }
    return fib(n - 1) + fib(n - 2);
}
"#,
        },
        Fixture {
            name: "08_struct_decl_and_lit",
            source: r#"
struct Point {
    x: f32,
    y: f32,
}

fn make() -> Point {
    return Point { x: 1.0, y: 2.0 };
}
"#,
        },
        Fixture {
            name: "09_struct_field_access",
            source: r#"
struct Vec2 {
    x: f32,
    y: f32,
}

fn dot(a: Vec2, b: Vec2) -> f32 {
    return a.x * b.x + a.y * b.y;
}
"#,
        },
        Fixture {
            name: "10_nested_struct",
            source: r#"
struct Inner { v: i32 }
struct Outer { i: Inner, k: i32 }

fn pick(o: Outer) -> i32 {
    return o.i.v + o.k;
}
"#,
        },
        Fixture {
            name: "11_closure_simple",
            source: r#"
fn make_adder(k: i64) -> fn(i64) -> i64 {
    return |x: i64| x + k;
}
"#,
        },
        Fixture {
            name: "12_closure_arith",
            source: r#"
fn apply(f: fn(i64) -> i64, n: i64) -> i64 {
    return f(n) + f(n + 1);
}
"#,
        },
        Fixture {
            name: "13_logical_ops",
            source: r#"
fn xor(a: bool, b: bool) -> bool {
    return (a || b) && !(a && b);
}
"#,
        },
        Fixture {
            name: "14_const_decl",
            source: r#"
const TAU: f64 = 6.283185307179586;
const TWO: i64 = 2;

fn doubled(n: i64) -> i64 {
    return n * TWO;
}
"#,
        },
        Fixture {
            name: "15_ecs_query_sugar",
            source: r#"
struct Position { x: f32, y: f32 }
struct Velocity { x: f32, y: f32 }

fn movement(q: Query<Position>, dt: Res<f32>) -> nil {
    return;
}
"#,
        },
        Fixture {
            name: "16_ecs_resmut",
            source: r#"
struct Score { value: i32 }

fn award(s: ResMut<Score>, e: Entity) -> nil {
    return;
}
"#,
        },
        Fixture {
            name: "17_nested_block_value",
            source: r#"
fn pick(b: bool) -> i64 {
    let v: i64 = if b { 100 } else { 200 };
    return v;
}
"#,
        },
        Fixture {
            name: "18_comparison_chain",
            source: r#"
fn clip(x: i64, lo: i64, hi: i64) -> i64 {
    if x < lo {
        return lo;
    }
    if x > hi {
        return hi;
    }
    return x;
}
"#,
        },
        Fixture {
            name: "19_string_literal",
            source: r#"
fn greet() -> str {
    return "hello, sliced engine";
}
"#,
        },
        Fixture {
            name: "20_mixed_program",
            source: r#"
struct Counter { value: i64 }

const INCREMENT: i64 = 3;

fn step(c: Counter) -> Counter {
    return Counter { value: c.value + INCREMENT };
}

fn main() -> i64 {
    let mut c: Counter = Counter { value: 0 };
    let mut i: i64 = 0;
    while i < 5 {
        c = step(c);
        i = i + 1;
    }
    return c.value;
}
"#,
        },
        // ADR-060 aggregate-ops codegen fixtures. The opcodes 0x70-0x7B
        // landed in Phase 5 PR 6; codegen for the AST shapes
        // ArrayLit / MapLit / StructLit / Field / Index / Closure was
        // wired in the same commit. The compile-parity golden takes a
        // digest over the IR text — these fixtures pin that text.
        Fixture {
            name: "21_array_literal",
            source: r#"
fn main() -> i64 {
    let xs: Array<i64> = [10, 20, 30];
    return xs[0] + xs[1] + xs[2];
}
"#,
        },
        Fixture {
            name: "22_array_indexing",
            source: r#"
fn at(xs: Array<i64>, i: i64) -> i64 {
    return xs[i];
}
"#,
        },
        Fixture {
            name: "23_map_literal",
            source: r#"
fn main() -> i64 {
    let m: Map<str, i64> = ["a" => 1, "b" => 2, "c" => 3];
    return m["a"] + m["b"] + m["c"];
}
"#,
        },
        Fixture {
            name: "24_closure_with_capture",
            source: r#"
fn make_adder(k: i64) -> fn(i64) -> i64 {
    let f = |x: i64| x + k;
    return f;
}
"#,
        },
    ];
    v.sort_by_key(|f| f.name);
    v
}
