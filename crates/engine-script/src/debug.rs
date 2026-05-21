//! sli script debugger (spec XII, ADR-036).
//!
//! Owns the in-process breakpoint table and the TRAP-opcode
//! patch/restore. Breakpoint identity is `(file_id, line)`; the
//! debugger maps those to `(function_id, byte_offset)` by walking the
//! compiled module's `line_for_pc` side-table.
//!
//! Step over / into / out fire one-shot breakpoints at the next
//! source line for the appropriate depth. Watch expressions evaluate
//! against the paused frame; they are validated by
//! [`crate::watch_expr`] to be side-effect-free before they run.

use crate::bytecode::{Module, Opcode};
use crate::vm::{CallFrame, Value};
use std::collections::BTreeMap;

/// Stable id assigned to one breakpoint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BreakpointId(pub u32);

/// One installed breakpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Breakpoint {
    /// File id (as the compiler knows it).
    pub file_id: u32,
    /// 1-based line.
    pub line: u32,
    /// Optional condition expression. Validated by
    /// [`crate::watch_expr`] to be side-effect-free.
    pub condition: Option<String>,
    /// Optional hit count: only fires every Nth time.
    pub hit_count: Option<u32>,
    /// Patched function id (resolved at install time).
    pub function_id: u16,
    /// Patched byte offset within that function.
    pub pc: usize,
    /// Original opcode byte we replaced with TRAP.
    pub original: u8,
    /// Times this breakpoint has been hit.
    pub hits: u32,
}

/// Breakpoint table + TRAP patcher (ADR-036).
#[derive(Debug, Default)]
pub struct Debugger {
    next_id: u32,
    breakpoints: BTreeMap<BreakpointId, Breakpoint>,
}

impl Debugger {
    /// An empty debugger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs a line breakpoint by patching the corresponding opcode
    /// in the module's code with [`Opcode::Trap`]. Returns the new id
    /// or an error if the line has no opcode boundary.
    pub fn set_breakpoint(
        &mut self,
        module: &mut Module,
        file_id: u32,
        line: u32,
    ) -> Result<BreakpointId, String> {
        // Find the first opcode in any function whose `line_for_pc`
        // matches.
        for (fn_id, f) in module.functions.iter_mut().enumerate() {
            let mut pc = 0;
            while pc < f.code.len() {
                let byte = f.code[pc];
                if let Some(op) = Opcode::from_u8(byte) {
                    if f.line_for_pc.get(pc).copied() == Some(line) {
                        let original = byte;
                        f.code[pc] = Opcode::Trap as u8;
                        let id = BreakpointId(self.next_id);
                        self.next_id += 1;
                        self.breakpoints.insert(
                            id,
                            Breakpoint {
                                file_id,
                                line,
                                condition: None,
                                hit_count: None,
                                function_id: fn_id as u16,
                                pc,
                                original,
                                hits: 0,
                            },
                        );
                        return Ok(id);
                    }
                    pc += op.instr_len(&f.code, pc);
                } else {
                    pc += 1;
                }
            }
        }
        Err(format!("no opcode at line {line}"))
    }

    /// Removes a breakpoint, restoring the original opcode byte.
    pub fn clear_breakpoint(
        &mut self,
        module: &mut Module,
        id: BreakpointId,
    ) -> Result<(), String> {
        let bp = self
            .breakpoints
            .remove(&id)
            .ok_or_else(|| format!("unknown breakpoint {}", id.0))?;
        if let Some(f) = module.functions.get_mut(bp.function_id as usize)
            && let Some(slot) = f.code.get_mut(bp.pc)
        {
            *slot = bp.original;
        }
        Ok(())
    }

    /// Borrows a breakpoint.
    pub fn breakpoint(&self, id: BreakpointId) -> Option<&Breakpoint> {
        self.breakpoints.get(&id)
    }

    /// Records that the breakpoint at `(function_id, pc)` was hit.
    /// Returns its id, if any. Increments hit count for hit-count
    /// gating.
    pub fn record_hit(&mut self, function_id: u16, pc: usize) -> Option<BreakpointId> {
        let id = self
            .breakpoints
            .iter()
            .find_map(|(id, bp)| (bp.function_id == function_id && bp.pc == pc).then_some(*id))?;
        if let Some(bp) = self.breakpoints.get_mut(&id) {
            bp.hits += 1;
        }
        Some(id)
    }

    /// Iterates installed breakpoints.
    pub fn iter(&self) -> impl Iterator<Item = (BreakpointId, &Breakpoint)> {
        self.breakpoints.iter().map(|(id, bp)| (*id, bp))
    }

    /// Number of installed breakpoints.
    pub fn len(&self) -> usize {
        self.breakpoints.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.breakpoints.is_empty()
    }

    /// Re-arms every breakpoint after a hot-reload. Walks the swapped
    /// module's `line_for_pc` to recompute pc offsets for each
    /// `(file_id, line)` pair; breakpoints whose lines no longer map
    /// to opcode boundaries are reported in the returned list and
    /// dropped from the table.
    pub fn rearm(&mut self, module: &mut Module) -> Vec<BreakpointId> {
        let mut dropped = Vec::new();
        let mut new_bps: BTreeMap<BreakpointId, Breakpoint> = BTreeMap::new();
        for (id, bp) in std::mem::take(&mut self.breakpoints) {
            let mut found = false;
            for (fn_id, f) in module.functions.iter_mut().enumerate() {
                let mut pc = 0;
                while pc < f.code.len() {
                    let byte = f.code[pc];
                    if let Some(op) = Opcode::from_u8(byte) {
                        if f.line_for_pc.get(pc).copied() == Some(bp.line) {
                            let original = byte;
                            f.code[pc] = Opcode::Trap as u8;
                            let mut nbp = bp.clone();
                            nbp.function_id = fn_id as u16;
                            nbp.pc = pc;
                            nbp.original = original;
                            new_bps.insert(id, nbp);
                            found = true;
                            break;
                        }
                        pc += op.instr_len(&f.code, pc);
                    } else {
                        pc += 1;
                    }
                }
                if found {
                    break;
                }
            }
            if !found {
                dropped.push(id);
            }
        }
        self.breakpoints = new_bps;
        dropped
    }
}

/// Read-only view of a paused frame, handed to the `ListLocals` /
/// `ExpandValue` debugger commands.
pub struct PausedFrame<'a> {
    /// Top-of-stack frame the debugger paused on.
    pub frame: &'a CallFrame,
    /// Source-readable locals — `(name, register, current value)`. The
    /// codegen does not yet emit a debug-info local table, so the PR-3
    /// surface exposes registers by index.
    pub locals: Vec<(String, u8, Value)>,
}

impl<'a> PausedFrame<'a> {
    /// Constructs a snapshot from a live frame.
    pub fn new(frame: &'a CallFrame) -> Self {
        let mut locals = Vec::with_capacity(frame.registers.len());
        for (i, v) in frame.registers.iter().enumerate() {
            locals.push((format!("r{i}"), i as u8, v.clone()));
        }
        Self { frame, locals }
    }
}
