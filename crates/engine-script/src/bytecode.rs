//! sli register bytecode — on-disk and in-memory representation.
//!
//! One byte per opcode, immediate operands packed little-endian. Registers
//! are addressed as `u8` (256 per frame, spec V.3); a function with more
//! than 256 live values triggers a verification error.
//!
//! The reserved opcode `TRAP = 0xFF` is the debugger's breakpoint patch
//! (PR 3). The VM treats it as a `signal-and-pause`; the verifier (this
//! crate's `verify.rs`) rejects every other path to `0xFF` so user code
//! cannot collide with it. The architectural impossibility is established
//! at four layers: (1) the [`Opcode`] enum has `TRAP` as its only `0xFF`
//! discriminant, (2) the `const _: () = assert!` below pins the layout,
//! (3) the verifier checks `byte != 0xFF` on every code byte, (4) the
//! `tests/codegen_no_trap.rs` fuzz oracle compiles a synthetic corpus
//! and asserts no emitted byte equals `0xFF`. ADR-035 records the design.

use crate::vm::Value;

/// One opcode. `#[repr(u8)]` so the in-memory byte equals the on-disk byte.
///
/// `TRAP = 0xFF` is the only `0xFF` discriminant. Codegen never emits
/// `TRAP`; only the PR-3 debugger may patch it in (see `debug.rs`).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opcode {
    /// No-op. Used as filler in patched debugger sites.
    Nop = 0x00,
    /// `dst = nil` — `dst:u8`.
    ConstNil = 0x01,
    /// `dst = true` — `dst:u8`.
    ConstTrue = 0x02,
    /// `dst = false` — `dst:u8`.
    ConstFalse = 0x03,
    /// `dst = const_pool[idx]` (integer) — `dst:u8 idx:u16le`.
    ConstInt = 0x04,
    /// `dst = const_pool[idx]` (float) — `dst:u8 idx:u16le`.
    ConstFloat = 0x05,
    /// `dst = const_pool[idx]` (string) — `dst:u8 idx:u16le`. Decodes
    /// into an `Arc<str>` reference held by the value.
    ConstStr = 0x06,
    /// `dst = src` — `dst:u8 src:u8`.
    Move = 0x07,

    /// `dst = a + b` (int or float; tag-dispatched).
    Add = 0x10,
    /// `dst = a - b`.
    Sub = 0x11,
    /// `dst = a * b`.
    Mul = 0x12,
    /// `dst = a / b`.
    Div = 0x13,
    /// `dst = a % b`.
    Mod = 0x14,
    /// `dst = -a`.
    Neg = 0x15,

    /// `dst = a == b`.
    Eq = 0x20,
    /// `dst = a != b`.
    Ne = 0x21,
    /// `dst = a < b`.
    Lt = 0x22,
    /// `dst = a <= b`.
    Le = 0x23,
    /// `dst = a > b`.
    Gt = 0x24,
    /// `dst = a >= b`.
    Ge = 0x25,

    /// `dst = !a`.
    Not = 0x30,
    /// `dst = a && b` (eager — no short-circuit at this level).
    And = 0x31,
    /// `dst = a || b`.
    Or = 0x32,

    /// Unconditional branch — `offset:i16le` relative to next pc.
    Jmp = 0x40,
    /// Branch if `cond == false` — `cond:u8 offset:i16le`.
    JmpIfFalse = 0x41,
    /// Branch if `cond == true` — `cond:u8 offset:i16le`.
    JmpIfTrue = 0x42,

    /// Call sli function at `fn_idx` — `dst:u8 fn_idx:u16le n:u8 arg0:u8 .. argN-1:u8`.
    Call = 0x50,
    /// Call FFI function at `ffi_idx` — same layout as `Call`.
    FfiCall = 0x51,

    /// `return nil`.
    ReturnNil = 0x60,
    /// `return src` — `src:u8`.
    ReturnVal = 0x61,

    /// Allocate a new array from `n` register values. The runtime
    /// resolves each register, packs the values into an `Obj::Array`,
    /// and stores the resulting handle in `dst`. Layout:
    /// `dst:u8 n:u8 arg0:u8 .. argN-1:u8` (ADR-060).
    ArrayNew = 0x70,
    /// `dst = arr[idx]` — `dst:u8 arr:u8 idx:u8`. `idx` is the
    /// integer register; runtime traps on negative or out-of-bounds.
    ArrayGet = 0x71,
    /// `arr[idx] = src` — `arr:u8 idx:u8 src:u8`. Fires the write
    /// barrier when `arr` is old-gen and `src` is a nursery handle.
    ArraySet = 0x72,
    /// `dst = arr.len()` — `dst:u8 arr:u8`. Result is an `Int`.
    ArrayLen = 0x73,
    /// Allocate an empty map — `dst:u8`. Populated by subsequent
    /// `MapSet` opcodes.
    MapNew = 0x74,
    /// `dst = map[key]` — `dst:u8 map:u8 key:u8`. `key` register must
    /// hold a `Str`; runtime trap otherwise. Missing key yields `Nil`.
    MapGet = 0x75,
    /// `map[key] = src` — `map:u8 key:u8 src:u8`. Fires the write
    /// barrier.
    MapSet = 0x76,
    /// Allocate an empty struct — `dst:u8`. Populated by `StructSet`.
    StructNew = 0x77,
    /// `dst = strct.<name>` — `dst:u8 strct:u8 name_ki:u16le`.
    /// `name_ki` indexes the const pool's `Str` entry holding the
    /// field name.
    StructGet = 0x78,
    /// `strct.<name> = src` — `strct:u8 name_ki:u16le src:u8`. Fires
    /// the write barrier.
    StructSet = 0x79,
    /// Build a closure object — `dst:u8 fn_idx:u16le n:u8 up0:u8 ..
    /// upN-1:u8`. The `n` upvalues are captured by value.
    ClosureMake = 0x7A,
    /// Call a closure — `dst:u8 cls:u8 n:u8 arg0:u8 .. argN-1:u8`.
    /// The closure's captured upvalues are passed as registers 0..k
    /// of the callee frame; user args occupy `k..k+n`.
    CallClosure = 0x7B,

    /// Debugger breakpoint marker. Never emitted by codegen.
    Trap = 0xFF,
}

// Architectural impossibility: TRAP must be the only 0xFF.
// If anyone adds a second variant whose discriminant happens to be 0xFF,
// the next `const _` fires at compile time.
const _: () = {
    assert!(Opcode::Trap as u8 == 0xFF);
};

impl Opcode {
    /// Decodes a byte into an opcode. Returns `None` for unknown bytes
    /// (including `0xFE` and any other unassigned slot).
    pub fn from_u8(b: u8) -> Option<Self> {
        Some(match b {
            0x00 => Self::Nop,
            0x01 => Self::ConstNil,
            0x02 => Self::ConstTrue,
            0x03 => Self::ConstFalse,
            0x04 => Self::ConstInt,
            0x05 => Self::ConstFloat,
            0x06 => Self::ConstStr,
            0x07 => Self::Move,
            0x10 => Self::Add,
            0x11 => Self::Sub,
            0x12 => Self::Mul,
            0x13 => Self::Div,
            0x14 => Self::Mod,
            0x15 => Self::Neg,
            0x20 => Self::Eq,
            0x21 => Self::Ne,
            0x22 => Self::Lt,
            0x23 => Self::Le,
            0x24 => Self::Gt,
            0x25 => Self::Ge,
            0x30 => Self::Not,
            0x31 => Self::And,
            0x32 => Self::Or,
            0x40 => Self::Jmp,
            0x41 => Self::JmpIfFalse,
            0x42 => Self::JmpIfTrue,
            0x50 => Self::Call,
            0x51 => Self::FfiCall,
            0x60 => Self::ReturnNil,
            0x61 => Self::ReturnVal,
            0x70 => Self::ArrayNew,
            0x71 => Self::ArrayGet,
            0x72 => Self::ArraySet,
            0x73 => Self::ArrayLen,
            0x74 => Self::MapNew,
            0x75 => Self::MapGet,
            0x76 => Self::MapSet,
            0x77 => Self::StructNew,
            0x78 => Self::StructGet,
            0x79 => Self::StructSet,
            0x7A => Self::ClosureMake,
            0x7B => Self::CallClosure,
            0xFF => Self::Trap,
            _ => return None,
        })
    }

    /// On-wire byte length of this opcode and its inline operands, given
    /// the byte stream starting at the opcode byte. For variable-length
    /// opcodes (`Call`, `FfiCall`), pass the full code slice so the
    /// argument count can be read.
    pub fn instr_len(self, code: &[u8], pc: usize) -> usize {
        match self {
            Self::Nop | Self::ReturnNil | Self::Trap => 1,
            Self::ConstNil | Self::ConstTrue | Self::ConstFalse | Self::ReturnVal => 1 + 1,
            Self::Move | Self::Not | Self::Neg => 1 + 2,
            Self::ConstInt | Self::ConstFloat | Self::ConstStr => 1 + 1 + 2,
            Self::Add
            | Self::Sub
            | Self::Mul
            | Self::Div
            | Self::Mod
            | Self::Eq
            | Self::Ne
            | Self::Lt
            | Self::Le
            | Self::Gt
            | Self::Ge
            | Self::And
            | Self::Or => 1 + 3,
            Self::Jmp => 1 + 2,
            Self::JmpIfFalse | Self::JmpIfTrue => 1 + 1 + 2,
            Self::Call | Self::FfiCall => {
                // `dst:u8 fn_idx:u16 n:u8 args:[u8; n]`
                let n = code.get(pc + 4).copied().unwrap_or(0) as usize;
                1 + 1 + 2 + 1 + n
            }
            // Aggregate opcodes (ADR-060).
            Self::ArrayNew => {
                // `dst:u8 n:u8 args:[u8; n]`
                let n = code.get(pc + 2).copied().unwrap_or(0) as usize;
                1 + 1 + 1 + n
            }
            Self::ArrayGet | Self::ArraySet => 1 + 3,
            Self::ArrayLen => 1 + 2,
            Self::MapNew | Self::StructNew => 1 + 1,
            Self::MapGet | Self::MapSet => 1 + 3,
            Self::StructGet | Self::StructSet => 1 + 1 + 2 + 1,
            Self::ClosureMake => {
                // `dst:u8 fn_idx:u16 n:u8 ups:[u8; n]`
                let n = code.get(pc + 4).copied().unwrap_or(0) as usize;
                1 + 1 + 2 + 1 + n
            }
            Self::CallClosure => {
                // `dst:u8 cls:u8 n:u8 args:[u8; n]`
                let n = code.get(pc + 3).copied().unwrap_or(0) as usize;
                1 + 1 + 1 + 1 + n
            }
        }
    }
}

/// Entry in the const pool. Floats use `to_bits` for deterministic
/// equality and hashing.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Const {
    /// 64-bit signed integer.
    Int(i64),
    /// 64-bit float (bit pattern).
    Float(u64),
    /// UTF-8 string.
    Str(String),
}

/// Compiled bytecode of one function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionBytecode {
    /// Function name — used by `Call` dispatch and the debugger.
    pub name: String,
    /// Number of arguments. Arg registers occupy `r0..r(arity)`.
    pub arity: u8,
    /// Highest register index reached during codegen, +1.
    pub max_register: u8,
    /// Code bytes. Verifier-checked before execution.
    pub code: Vec<u8>,
    /// Per-instruction source line, indexed by code byte offset. Sparse
    /// — only opcode bytes carry a line; immediate-operand bytes are
    /// implicit `None`.
    pub line_for_pc: Vec<u32>,
}

/// One compiled module: const pool, function table, function name index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Module {
    /// Const pool (sorted insertion via [`ModuleBuilder`]; index-stable
    /// across compilations of the same source).
    pub constants: Vec<Const>,
    /// Functions in declaration order.
    pub functions: Vec<FunctionBytecode>,
    /// `name -> index in `functions``.
    pub function_index: Vec<(String, u16)>,
}

impl Module {
    /// Looks up a function by name.
    pub fn function_id(&self, name: &str) -> Option<u16> {
        self.function_index
            .iter()
            .find_map(|(n, id)| (n == name).then_some(*id))
    }

    /// Borrows a function by index.
    pub fn function(&self, id: u16) -> Option<&FunctionBytecode> {
        self.functions.get(id as usize)
    }
}

/// Builder used by the AST → bytecode lowering pass.
#[derive(Debug, Default)]
pub struct ModuleBuilder {
    module: Module,
    const_cache: Vec<(Const, u16)>,
}

impl ModuleBuilder {
    /// Constructs an empty module.
    pub fn new() -> Self {
        Self::default()
    }

    /// Interns a const, returning its 16-bit pool index. Identical
    /// constants share an index — string deduplication is one of the
    /// most common savings.
    pub fn intern(&mut self, c: Const) -> u16 {
        if let Some((_, idx)) = self.const_cache.iter().find(|(cc, _)| cc == &c) {
            return *idx;
        }
        let idx = self.module.constants.len() as u16;
        self.module.constants.push(c.clone());
        self.const_cache.push((c, idx));
        idx
    }

    /// Pushes a compiled function and records its name in the index.
    pub fn push_function(&mut self, f: FunctionBytecode) -> u16 {
        let id = self.module.functions.len() as u16;
        self.module.function_index.push((f.name.clone(), id));
        self.module.functions.push(f);
        id
    }

    /// Finalises the module.
    pub fn build(self) -> Module {
        self.module
    }
}

// --- on-disk encoding -------------------------------------------------------

const MAGIC: &[u8; 8] = b"ENGNSLI1";
const FORMAT_VERSION: u32 = 1;

/// Encodes `module` into a deterministic byte stream.
///
/// Layout: `MAGIC[8] | version:u32le | const_count:u32le |
/// (tag:u8 + payload)* | fn_count:u32le | (name_len:u32le name name_bytes
/// arity:u8 max_register:u8 code_len:u32le code line_count:u32le lines)*`.
pub fn encode(module: &Module) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(&(module.constants.len() as u32).to_le_bytes());
    for c in &module.constants {
        match c {
            Const::Int(v) => {
                buf.push(0);
                buf.extend_from_slice(&v.to_le_bytes());
            }
            Const::Float(b) => {
                buf.push(1);
                buf.extend_from_slice(&b.to_le_bytes());
            }
            Const::Str(s) => {
                buf.push(2);
                buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
                buf.extend_from_slice(s.as_bytes());
            }
        }
    }
    buf.extend_from_slice(&(module.functions.len() as u32).to_le_bytes());
    for f in &module.functions {
        buf.extend_from_slice(&(f.name.len() as u32).to_le_bytes());
        buf.extend_from_slice(f.name.as_bytes());
        buf.push(f.arity);
        buf.push(f.max_register);
        buf.extend_from_slice(&(f.code.len() as u32).to_le_bytes());
        buf.extend_from_slice(&f.code);
        buf.extend_from_slice(&(f.line_for_pc.len() as u32).to_le_bytes());
        for l in &f.line_for_pc {
            buf.extend_from_slice(&l.to_le_bytes());
        }
    }
    buf
}

/// Decodes a byte stream produced by [`encode`]. Returns the module or
/// the first byte offset at which decoding failed.
pub fn decode(bytes: &[u8]) -> Result<Module, BytecodeError> {
    let mut cur = Cursor::new(bytes);
    let magic = cur.take(8)?;
    if magic != MAGIC {
        return Err(BytecodeError::BadMagic);
    }
    let version = cur.u32()?;
    if version != FORMAT_VERSION {
        return Err(BytecodeError::UnsupportedVersion(version));
    }
    let const_count = cur.u32()? as usize;
    let mut constants = Vec::with_capacity(const_count);
    for _ in 0..const_count {
        let tag = cur.u8()?;
        let c = match tag {
            0 => {
                let bytes = cur.take(8)?;
                Const::Int(i64::from_le_bytes(bytes.try_into().unwrap()))
            }
            1 => {
                let bytes = cur.take(8)?;
                Const::Float(u64::from_le_bytes(bytes.try_into().unwrap()))
            }
            2 => {
                let len = cur.u32()? as usize;
                let bytes = cur.take(len)?;
                Const::Str(String::from_utf8(bytes.to_vec()).map_err(|_| BytecodeError::BadUtf8)?)
            }
            _ => return Err(BytecodeError::UnknownConstTag(tag)),
        };
        constants.push(c);
    }
    let fn_count = cur.u32()? as usize;
    let mut functions = Vec::with_capacity(fn_count);
    let mut function_index = Vec::with_capacity(fn_count);
    for id in 0..fn_count {
        let name_len = cur.u32()? as usize;
        let name_bytes = cur.take(name_len)?;
        let name = String::from_utf8(name_bytes.to_vec()).map_err(|_| BytecodeError::BadUtf8)?;
        let arity = cur.u8()?;
        let max_register = cur.u8()?;
        let code_len = cur.u32()? as usize;
        let code = cur.take(code_len)?.to_vec();
        let line_count = cur.u32()? as usize;
        let mut line_for_pc = Vec::with_capacity(line_count);
        for _ in 0..line_count {
            line_for_pc.push(cur.u32()?);
        }
        function_index.push((name.clone(), id as u16));
        functions.push(FunctionBytecode {
            name,
            arity,
            max_register,
            code,
            line_for_pc,
        });
    }
    Ok(Module {
        constants,
        functions,
        function_index,
    })
}

/// Why a bytecode buffer could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BytecodeError {
    /// Magic bytes did not match.
    BadMagic,
    /// Format version is newer than this build understands.
    UnsupportedVersion(u32),
    /// A string blob was not valid UTF-8.
    BadUtf8,
    /// A const-pool tag byte was not one of the known values.
    UnknownConstTag(u8),
    /// Reached EOF while expecting more bytes.
    Truncated,
}

impl std::fmt::Display for BytecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMagic => write!(f, "bad magic"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported format version {v}"),
            Self::BadUtf8 => write!(f, "invalid UTF-8 in string constant"),
            Self::UnknownConstTag(t) => write!(f, "unknown const-pool tag {t}"),
            Self::Truncated => write!(f, "truncated bytecode"),
        }
    }
}

impl std::error::Error for BytecodeError {}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], BytecodeError> {
        if self.pos + n > self.bytes.len() {
            return Err(BytecodeError::Truncated);
        }
        let out = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, BytecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, BytecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes(b.try_into().unwrap()))
    }
}

/// Round-trips a module through `encode` / `decode`. Returns `true` if
/// the decoded module equals the original — used by tests.
pub fn roundtrip_eq(module: &Module) -> bool {
    let bytes = encode(module);
    match decode(&bytes) {
        Ok(m) => &m == module,
        Err(_) => false,
    }
}

// `vm::Value` is referenced from the FFI marshal layer; importing it here
// keeps the visible type surface tidy.
#[allow(dead_code)]
fn _value_use() -> Option<Value> {
    None
}
