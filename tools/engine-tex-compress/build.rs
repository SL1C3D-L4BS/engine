//! Workaround for intel_tex_2 0.5: it builds `ispc_texcomp_astc.cpp` as a
//! static C++ object but does not declare a `link-lib=stdc++` directive, so
//! linking the final binary on Linux fails with an undefined reference to
//! `__gxx_personality_v0`. Linking the C++ standard library here closes the
//! gap. macOS / Windows toolchains link the C++ runtime by default; only
//! `target_os = "linux"` needs the hint.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-lib=stdc++");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
