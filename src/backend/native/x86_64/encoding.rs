// backend/native/x86_64/encoding.rs — x86-64 instruction encoding tables.
//
// Provides opcode lookup, REX prefix generation, ModR/M byte encoding,
// and full instruction serialization for the builtin assembler.

#![allow(dead_code)]

/// REX prefix byte construction.
/// REX = 0100WRXB where:
///   W = operand size (1 = 64-bit)
///   R = ModRM.reg extension
///   X = SIB.index extension
///   B = ModRM.rm or SIB.base extension
pub const REX_BASE: u8 = 0x40;
pub const REX_W: u8 = 0x08; // 64-bit operand size
pub const REX_R: u8 = 0x04; // ModRM.reg extension
pub const REX_X: u8 = 0x02; // SIB.index extension
pub const REX_B: u8 = 0x01; // ModRM.rm extension

/// Build a REX prefix byte from flags.
pub fn rex(w: bool, r: bool, x: bool, b: bool) -> u8 {
    let mut byte = REX_BASE;
    if w { byte |= REX_W; }
    if r { byte |= REX_R; }
    if x { byte |= REX_X; }
    if b { byte |= REX_B; }
    byte
}

/// Build a ModR/M byte.
/// mod (2 bits) | reg (3 bits) | rm (3 bits)
pub fn modrm(mode: u8, reg: u8, rm: u8) -> u8 {
    ((mode & 0x3) << 6) | ((reg & 0x7) << 3) | (rm & 0x7)
}

/// Build a SIB byte.
/// scale (2 bits) | index (3 bits) | base (3 bits)
pub fn sib(scale: u8, index: u8, base: u8) -> u8 {
    ((scale & 0x3) << 6) | ((index & 0x7) << 3) | (base & 0x7)
}

/// x86-64 register encoding (low 3 bits, with possible REX.B or REX.R).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegCode {
    /// Low 3 bits of register number.
    pub code: u8,
    /// Whether the register requires REX extension (bit 4).
    pub ext: bool,
}

impl RegCode {
    pub const fn new(code: u8, ext: bool) -> Self {
        Self { code, ext }
    }
}

/// Map register name to encoding.
pub fn reg_code(name: &str) -> Option<RegCode> {
    match name {
        // 64-bit
        "rax" | "eax" | "ax" | "al" => Some(RegCode::new(0, false)),
        "rcx" | "ecx" | "cx" | "cl" => Some(RegCode::new(1, false)),
        "rdx" | "edx" | "dx" | "dl" => Some(RegCode::new(2, false)),
        "rbx" | "ebx" | "bx" | "bl" => Some(RegCode::new(3, false)),
        "rsp" | "esp" | "sp" | "spl" | "ah" => Some(RegCode::new(4, false)),
        "rbp" | "ebp" | "bp" | "bpl" | "ch" => Some(RegCode::new(5, false)),
        "rsi" | "esi" | "si" | "sil" | "dh" => Some(RegCode::new(6, false)),
        "rdi" | "edi" | "di" | "dil" | "bh" => Some(RegCode::new(7, false)),
        "r8" | "r8d" | "r8w" | "r8b" => Some(RegCode::new(0, true)),
        "r9" | "r9d" | "r9w" | "r9b" => Some(RegCode::new(1, true)),
        "r10" | "r10d" | "r10w" | "r10b" => Some(RegCode::new(2, true)),
        "r11" | "r11d" | "r11w" | "r11b" => Some(RegCode::new(3, true)),
        "r12" | "r12d" | "r12w" | "r12b" => Some(RegCode::new(4, true)),
        "r13" | "r13d" | "r13w" | "r13b" => Some(RegCode::new(5, true)),
        "r14" | "r14d" | "r14w" | "r14b" => Some(RegCode::new(6, true)),
        "r15" | "r15d" | "r15w" | "r15b" => Some(RegCode::new(7, true)),
        // XMM registers
        "xmm0" => Some(RegCode::new(0, false)),
        "xmm1" => Some(RegCode::new(1, false)),
        "xmm2" => Some(RegCode::new(2, false)),
        "xmm3" => Some(RegCode::new(3, false)),
        "xmm4" => Some(RegCode::new(4, false)),
        "xmm5" => Some(RegCode::new(5, false)),
        "xmm6" => Some(RegCode::new(6, false)),
        "xmm7" => Some(RegCode::new(7, false)),
        "xmm8" => Some(RegCode::new(0, true)),
        "xmm9" => Some(RegCode::new(1, true)),
        "xmm10" => Some(RegCode::new(2, true)),
        "xmm11" => Some(RegCode::new(3, true)),
        "xmm12" => Some(RegCode::new(4, true)),
        "xmm13" => Some(RegCode::new(5, true)),
        "xmm14" => Some(RegCode::new(6, true)),
        "xmm15" => Some(RegCode::new(7, true)),
        _ => None,
    }
}

/// Operand size (for choosing between byte/word/dword/qword encoding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpSize {
    Byte,  // 8-bit
    Word,  // 16-bit
    Dword, // 32-bit
    Qword, // 64-bit
}

impl OpSize {
    /// Number of bytes.
    pub fn bytes(self) -> usize {
        match self {
            OpSize::Byte => 1,
            OpSize::Word => 2,
            OpSize::Dword => 4,
            OpSize::Qword => 8,
        }
    }

    /// Does this need a REX.W prefix?
    pub fn needs_rex_w(self) -> bool {
        matches!(self, OpSize::Qword)
    }

    /// Does this need an operand size override prefix (0x66)?
    pub fn needs_66_prefix(self) -> bool {
        matches!(self, OpSize::Word)
    }
}

/// Instruction encoding helper — appends encoded bytes to a buffer.
pub struct Encoder {
    pub buf: Vec<u8>,
}

impl Encoder {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Emit raw bytes.
    pub fn emit_bytes(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Emit a single byte.
    pub fn emit_byte(&mut self, b: u8) {
        self.buf.push(b);
    }

    /// Emit a little-endian u16.
    pub fn emit_u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Emit a little-endian u32.
    pub fn emit_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Emit a little-endian u64.
    pub fn emit_u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Emit a signed 8-bit displacement.
    pub fn emit_imm8(&mut self, v: i8) {
        self.buf.push(v as u8);
    }

    /// Emit a signed 32-bit immediate/displacement.
    pub fn emit_imm32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    // ── Common instruction encodings ──────────────────────────────

    /// Encode NOP (0x90).
    pub fn encode_nop(&mut self) {
        self.emit_byte(0x90);
    }

    /// Encode SYSCALL (0x0F 0x05).
    pub fn encode_syscall(&mut self) {
        self.emit_byte(0x0F);
        self.emit_byte(0x05);
    }

    /// Encode RET (0xC3).
    pub fn encode_ret(&mut self) {
        self.emit_byte(0xC3);
    }

    /// Encode LEAVE (0xC9).
    pub fn encode_leave(&mut self) {
        self.emit_byte(0xC9);
    }

    /// Encode CQO (REX.W + 0x99) — sign-extend RAX → RDX:RAX.
    pub fn encode_cqto(&mut self) {
        self.emit_byte(rex(true, false, false, false));
        self.emit_byte(0x99);
    }

    /// Encode CDQ (0x99) — sign-extend EAX → EDX:EAX.
    pub fn encode_cltd(&mut self) {
        self.emit_byte(0x99);
    }

    /// Encode PUSH reg64 (50+rd).
    pub fn encode_push_reg(&mut self, reg: RegCode) {
        if reg.ext {
            self.emit_byte(rex(false, false, false, true));
        }
        self.emit_byte(0x50 + reg.code);
    }

    /// Encode POP reg64 (58+rd).
    pub fn encode_pop_reg(&mut self, reg: RegCode) {
        if reg.ext {
            self.emit_byte(rex(false, false, false, true));
        }
        self.emit_byte(0x58 + reg.code);
    }

    /// Encode MOV reg, reg (for 32/64-bit).
    pub fn encode_mov_reg_reg(&mut self, src: RegCode, dst: RegCode, size: OpSize) {
        if size.needs_66_prefix() {
            self.emit_byte(0x66);
        }
        let need_rex = size.needs_rex_w() || src.ext || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), src.ext, false, dst.ext));
        }
        // Opcode: 89 /r for MOV r/m, r or 8B /r for MOV r, r/m
        self.emit_byte(0x89);
        self.emit_byte(modrm(0b11, src.code, dst.code));
    }

    /// Encode MOV $imm32, reg (for 32-bit moves with zero-extension).
    pub fn encode_mov_imm32_reg(&mut self, imm: i32, dst: RegCode) {
        if dst.ext {
            self.emit_byte(rex(false, false, false, true));
        }
        self.emit_byte(0xB8 + dst.code);
        self.emit_imm32(imm);
    }

    /// Encode MOV $imm64, reg (REX.W + B8+rd io).
    pub fn encode_mov_imm64_reg(&mut self, imm: i64, dst: RegCode) {
        self.emit_byte(rex(true, false, false, dst.ext));
        self.emit_byte(0xB8 + dst.code);
        self.emit_u64(imm as u64);
    }

    /// Encode MOV [base + disp32], reg (load).
    pub fn encode_mov_mem_reg(&mut self, base: RegCode, disp: i32, dst: RegCode, size: OpSize) {
        if size.needs_66_prefix() {
            self.emit_byte(0x66);
        }
        let need_rex = size.needs_rex_w() || dst.ext || base.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), dst.ext, false, base.ext));
        }
        self.emit_byte(0x8B); // MOV r, r/m
        if disp == 0 && base.code != 5 {
            // [base] — mod=00
            self.emit_byte(modrm(0b00, dst.code, base.code));
            if base.code == 4 {
                self.emit_byte(sib(0, 4, 4)); // SIB for RSP-based addressing
            }
        } else if disp >= -128 && disp <= 127 {
            // [base + disp8] — mod=01
            self.emit_byte(modrm(0b01, dst.code, base.code));
            if base.code == 4 {
                self.emit_byte(sib(0, 4, 4));
            }
            self.emit_imm8(disp as i8);
        } else {
            // [base + disp32] — mod=10
            self.emit_byte(modrm(0b10, dst.code, base.code));
            if base.code == 4 {
                self.emit_byte(sib(0, 4, 4));
            }
            self.emit_imm32(disp);
        }
    }

    /// Encode MOV reg, [base + disp32] (store).
    pub fn encode_mov_reg_mem(&mut self, src: RegCode, base: RegCode, disp: i32, size: OpSize) {
        if size.needs_66_prefix() {
            self.emit_byte(0x66);
        }
        let need_rex = size.needs_rex_w() || src.ext || base.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), src.ext, false, base.ext));
        }
        self.emit_byte(0x89); // MOV r/m, r
        if disp == 0 && base.code != 5 {
            self.emit_byte(modrm(0b00, src.code, base.code));
            if base.code == 4 {
                self.emit_byte(sib(0, 4, 4));
            }
        } else if disp >= -128 && disp <= 127 {
            self.emit_byte(modrm(0b01, src.code, base.code));
            if base.code == 4 {
                self.emit_byte(sib(0, 4, 4));
            }
            self.emit_imm8(disp as i8);
        } else {
            self.emit_byte(modrm(0b10, src.code, base.code));
            if base.code == 4 {
                self.emit_byte(sib(0, 4, 4));
            }
            self.emit_imm32(disp);
        }
    }

    /// Encode SUB $imm32, reg.
    pub fn encode_sub_imm_reg(&mut self, imm: i32, dst: RegCode, size: OpSize) {
        if size.needs_66_prefix() {
            self.emit_byte(0x66);
        }
        let need_rex = size.needs_rex_w() || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), false, false, dst.ext));
        }
        if imm >= -128 && imm <= 127 {
            self.emit_byte(0x83); // /5: SUB r/m, imm8
            self.emit_byte(modrm(0b11, 5, dst.code));
            self.emit_imm8(imm as i8);
        } else {
            self.emit_byte(0x81); // /5: SUB r/m, imm32
            self.emit_byte(modrm(0b11, 5, dst.code));
            self.emit_imm32(imm);
        }
    }

    /// Encode ADD $imm32, reg.
    pub fn encode_add_imm_reg(&mut self, imm: i32, dst: RegCode, size: OpSize) {
        if size.needs_66_prefix() {
            self.emit_byte(0x66);
        }
        let need_rex = size.needs_rex_w() || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), false, false, dst.ext));
        }
        if imm >= -128 && imm <= 127 {
            self.emit_byte(0x83); // /0: ADD r/m, imm8
            self.emit_byte(modrm(0b11, 0, dst.code));
            self.emit_imm8(imm as i8);
        } else {
            self.emit_byte(0x81); // /0: ADD r/m, imm32
            self.emit_byte(modrm(0b11, 0, dst.code));
            self.emit_imm32(imm);
        }
    }

    /// Encode ADD reg, reg.
    pub fn encode_add_reg_reg(&mut self, src: RegCode, dst: RegCode, size: OpSize) {
        if size.needs_66_prefix() {
            self.emit_byte(0x66);
        }
        let need_rex = size.needs_rex_w() || src.ext || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), src.ext, false, dst.ext));
        }
        self.emit_byte(0x01); // ADD r/m, r
        self.emit_byte(modrm(0b11, src.code, dst.code));
    }

    /// Encode SUB reg, reg.
    pub fn encode_sub_reg_reg(&mut self, src: RegCode, dst: RegCode, size: OpSize) {
        if size.needs_66_prefix() {
            self.emit_byte(0x66);
        }
        let need_rex = size.needs_rex_w() || src.ext || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), src.ext, false, dst.ext));
        }
        self.emit_byte(0x29); // SUB r/m, r
        self.emit_byte(modrm(0b11, src.code, dst.code));
    }

    /// Encode IMUL reg, reg.
    pub fn encode_imul_reg_reg(&mut self, src: RegCode, dst: RegCode, size: OpSize) {
        if size.needs_66_prefix() {
            self.emit_byte(0x66);
        }
        let need_rex = size.needs_rex_w() || dst.ext || src.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), dst.ext, false, src.ext));
        }
        self.emit_bytes(&[0x0F, 0xAF]); // IMUL r, r/m
        self.emit_byte(modrm(0b11, dst.code, src.code));
    }

    /// Encode IDIV reg (signed division, rax / reg → rax with remainder in rdx).
    pub fn encode_idiv_reg(&mut self, src: RegCode, size: OpSize) {
        let need_rex = size.needs_rex_w() || src.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), false, false, src.ext));
        }
        self.emit_byte(0xF7); // /7
        self.emit_byte(modrm(0b11, 7, src.code));
    }

    /// Encode NEG reg.
    pub fn encode_neg_reg(&mut self, dst: RegCode, size: OpSize) {
        let need_rex = size.needs_rex_w() || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), false, false, dst.ext));
        }
        self.emit_byte(0xF7); // /3
        self.emit_byte(modrm(0b11, 3, dst.code));
    }

    /// Encode NOT reg.
    pub fn encode_not_reg(&mut self, dst: RegCode, size: OpSize) {
        let need_rex = size.needs_rex_w() || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), false, false, dst.ext));
        }
        self.emit_byte(0xF7); // /2
        self.emit_byte(modrm(0b11, 2, dst.code));
    }

    /// Encode CMP $imm32, reg.
    pub fn encode_cmp_imm_reg(&mut self, imm: i32, dst: RegCode, size: OpSize) {
        if size.needs_66_prefix() {
            self.emit_byte(0x66);
        }
        let need_rex = size.needs_rex_w() || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), false, false, dst.ext));
        }
        if imm >= -128 && imm <= 127 {
            self.emit_byte(0x83); // /7: CMP r/m, imm8
            self.emit_byte(modrm(0b11, 7, dst.code));
            self.emit_imm8(imm as i8);
        } else {
            self.emit_byte(0x81); // /7: CMP r/m, imm32
            self.emit_byte(modrm(0b11, 7, dst.code));
            self.emit_imm32(imm);
        }
    }

    /// Encode CMP reg, reg.
    pub fn encode_cmp_reg_reg(&mut self, src: RegCode, dst: RegCode, size: OpSize) {
        if size.needs_66_prefix() {
            self.emit_byte(0x66);
        }
        let need_rex = size.needs_rex_w() || src.ext || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), src.ext, false, dst.ext));
        }
        self.emit_byte(0x39); // CMP r/m, r
        self.emit_byte(modrm(0b11, src.code, dst.code));
    }

    /// Encode TEST reg, reg.
    pub fn encode_test_reg_reg(&mut self, src: RegCode, dst: RegCode, size: OpSize) {
        if size.needs_66_prefix() {
            self.emit_byte(0x66);
        }
        let need_rex = size.needs_rex_w() || src.ext || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), src.ext, false, dst.ext));
        }
        self.emit_byte(0x85); // TEST r/m, r
        self.emit_byte(modrm(0b11, src.code, dst.code));
    }

    /// Encode SETcc (conditional set byte).
    pub fn encode_setcc(&mut self, cc: CondCode, dst: RegCode) {
        if dst.ext {
            self.emit_byte(rex(false, false, false, true));
        }
        self.emit_bytes(&[0x0F, 0x90 + cc as u8]);
        self.emit_byte(modrm(0b11, 0, dst.code));
    }

    /// Encode Jcc rel32 (conditional jump with placeholder displacement).
    /// Returns the offset where the rel32 displacement starts (for patching).
    pub fn encode_jcc_rel32(&mut self, cc: CondCode) -> usize {
        self.emit_bytes(&[0x0F, 0x80 + cc as u8]);
        let patch_offset = self.buf.len();
        self.emit_imm32(0); // Placeholder
        patch_offset
    }

    /// Encode JMP rel32.
    /// Returns offset of the rel32 for patching.
    pub fn encode_jmp_rel32(&mut self) -> usize {
        self.emit_byte(0xE9);
        let patch_offset = self.buf.len();
        self.emit_imm32(0);
        patch_offset
    }

    /// Encode CALL rel32.
    /// Returns offset of the rel32 for patching.
    pub fn encode_call_rel32(&mut self) -> usize {
        self.emit_byte(0xE8);
        let patch_offset = self.buf.len();
        self.emit_imm32(0);
        patch_offset
    }

    /// Encode CALL *reg (indirect call).
    pub fn encode_call_indirect(&mut self, reg: RegCode) {
        if reg.ext {
            self.emit_byte(rex(false, false, false, true));
        }
        self.emit_byte(0xFF); // /2
        self.emit_byte(modrm(0b11, 2, reg.code));
    }

    /// Encode LEA [base + disp32], reg.
    pub fn encode_lea(&mut self, base: RegCode, disp: i32, dst: RegCode, size: OpSize) {
        let need_rex = size.needs_rex_w() || dst.ext || base.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), dst.ext, false, base.ext));
        }
        self.emit_byte(0x8D); // LEA r, m
        if disp >= -128 && disp <= 127 {
            self.emit_byte(modrm(0b01, dst.code, base.code));
            if base.code == 4 {
                self.emit_byte(sib(0, 4, 4));
            }
            self.emit_imm8(disp as i8);
        } else {
            self.emit_byte(modrm(0b10, dst.code, base.code));
            if base.code == 4 {
                self.emit_byte(sib(0, 4, 4));
            }
            self.emit_imm32(disp);
        }
    }

    /// Encode XOR reg, reg (commonly used to zero a register).
    pub fn encode_xor_reg_reg(&mut self, src: RegCode, dst: RegCode, size: OpSize) {
        if size.needs_66_prefix() {
            self.emit_byte(0x66);
        }
        let need_rex = size.needs_rex_w() || src.ext || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), src.ext, false, dst.ext));
        }
        self.emit_byte(0x31); // XOR r/m, r
        self.emit_byte(modrm(0b11, src.code, dst.code));
    }

    /// Encode AND reg, reg.
    pub fn encode_and_reg_reg(&mut self, src: RegCode, dst: RegCode, size: OpSize) {
        if size.needs_66_prefix() {
            self.emit_byte(0x66);
        }
        let need_rex = size.needs_rex_w() || src.ext || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), src.ext, false, dst.ext));
        }
        self.emit_byte(0x21); // AND r/m, r
        self.emit_byte(modrm(0b11, src.code, dst.code));
    }

    /// Encode OR reg, reg.
    pub fn encode_or_reg_reg(&mut self, src: RegCode, dst: RegCode, size: OpSize) {
        if size.needs_66_prefix() {
            self.emit_byte(0x66);
        }
        let need_rex = size.needs_rex_w() || src.ext || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), src.ext, false, dst.ext));
        }
        self.emit_byte(0x09); // OR r/m, r
        self.emit_byte(modrm(0b11, src.code, dst.code));
    }

    /// Encode SHL reg, CL.
    pub fn encode_shl_cl(&mut self, dst: RegCode, size: OpSize) {
        let need_rex = size.needs_rex_w() || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), false, false, dst.ext));
        }
        self.emit_byte(0xD3); // /4: SHL r/m, CL
        self.emit_byte(modrm(0b11, 4, dst.code));
    }

    /// Encode SHR reg, CL.
    pub fn encode_shr_cl(&mut self, dst: RegCode, size: OpSize) {
        let need_rex = size.needs_rex_w() || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), false, false, dst.ext));
        }
        self.emit_byte(0xD3); // /5: SHR r/m, CL
        self.emit_byte(modrm(0b11, 5, dst.code));
    }

    /// Encode SAR reg, CL.
    pub fn encode_sar_cl(&mut self, dst: RegCode, size: OpSize) {
        let need_rex = size.needs_rex_w() || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), false, false, dst.ext));
        }
        self.emit_byte(0xD3); // /7: SAR r/m, CL
        self.emit_byte(modrm(0b11, 7, dst.code));
    }

    /// Encode SHL reg, imm8.
    pub fn encode_shl_imm(&mut self, imm: u8, dst: RegCode, size: OpSize) {
        let need_rex = size.needs_rex_w() || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), false, false, dst.ext));
        }
        self.emit_byte(0xC1); // /4: SHL r/m, imm8
        self.emit_byte(modrm(0b11, 4, dst.code));
        self.emit_byte(imm);
    }

    /// Encode SHR reg, imm8.
    pub fn encode_shr_imm(&mut self, imm: u8, dst: RegCode, size: OpSize) {
        let need_rex = size.needs_rex_w() || dst.ext;
        if need_rex {
            self.emit_byte(rex(size.needs_rex_w(), false, false, dst.ext));
        }
        self.emit_byte(0xC1); // /5: SHR r/m, imm8
        self.emit_byte(modrm(0b11, 5, dst.code));
        self.emit_byte(imm);
    }

    /// Encode MOVZX (zero-extend from byte to dword/qword).
    pub fn encode_movzx_byte(&mut self, src: RegCode, dst: RegCode, dst_size: OpSize) {
        let need_rex = dst_size.needs_rex_w() || dst.ext || src.ext;
        if need_rex {
            self.emit_byte(rex(dst_size.needs_rex_w(), dst.ext, false, src.ext));
        }
        self.emit_bytes(&[0x0F, 0xB6]); // MOVZX r, r/m8
        self.emit_byte(modrm(0b11, dst.code, src.code));
    }

    /// Encode MOVSX (sign-extend from byte to dword/qword).
    pub fn encode_movsx_byte(&mut self, src: RegCode, dst: RegCode, dst_size: OpSize) {
        let need_rex = dst_size.needs_rex_w() || dst.ext || src.ext;
        if need_rex {
            self.emit_byte(rex(dst_size.needs_rex_w(), dst.ext, false, src.ext));
        }
        self.emit_bytes(&[0x0F, 0xBE]); // MOVSX r, r/m8
        self.emit_byte(modrm(0b11, dst.code, src.code));
    }

    /// Encode MOVSXD (sign-extend dword to qword via REX.W + 63).
    pub fn encode_movsxd(&mut self, src: RegCode, dst: RegCode) {
        self.emit_byte(rex(true, dst.ext, false, src.ext));
        self.emit_byte(0x63);
        self.emit_byte(modrm(0b11, dst.code, src.code));
    }

    /// Patch a rel32 at the given offset.
    pub fn patch_rel32(&mut self, offset: usize, target: usize) {
        let rel = (target as i64) - ((offset + 4) as i64);
        let rel32 = rel as i32;
        self.buf[offset..offset + 4].copy_from_slice(&rel32.to_le_bytes());
    }
}

/// Condition codes for Jcc/SETcc/CMOVcc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CondCode {
    O   = 0x0,  // Overflow
    NO  = 0x1,  // Not overflow
    B   = 0x2,  // Below (unsigned <)
    AE  = 0x3,  // Above or equal (unsigned >=)
    E   = 0x4,  // Equal
    NE  = 0x5,  // Not equal
    BE  = 0x6,  // Below or equal (unsigned <=)
    A   = 0x7,  // Above (unsigned >)
    S   = 0x8,  // Sign (negative)
    NS  = 0x9,  // Not sign
    P   = 0xA,  // Parity
    NP  = 0xB,  // Not parity
    L   = 0xC,  // Less (signed <)
    GE  = 0xD,  // Greater or equal (signed >=)
    LE  = 0xE,  // Less or equal (signed <=)
    G   = 0xF,  // Greater (signed >)
}

impl CondCode {
    /// Negate the condition.
    pub fn negate(self) -> CondCode {
        match self {
            CondCode::O => CondCode::NO,
            CondCode::NO => CondCode::O,
            CondCode::B => CondCode::AE,
            CondCode::AE => CondCode::B,
            CondCode::E => CondCode::NE,
            CondCode::NE => CondCode::E,
            CondCode::BE => CondCode::A,
            CondCode::A => CondCode::BE,
            CondCode::S => CondCode::NS,
            CondCode::NS => CondCode::S,
            CondCode::P => CondCode::NP,
            CondCode::NP => CondCode::P,
            CondCode::L => CondCode::GE,
            CondCode::GE => CondCode::L,
            CondCode::LE => CondCode::G,
            CondCode::G => CondCode::LE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rex_prefix() {
        assert_eq!(rex(false, false, false, false), 0x40);
        assert_eq!(rex(true, false, false, false), 0x48);
        assert_eq!(rex(true, true, false, false), 0x4C);
        assert_eq!(rex(false, false, false, true), 0x41);
    }

    #[test]
    fn test_modrm() {
        // Direct register-to-register: mod=11, reg=rax(0), rm=rcx(1)
        assert_eq!(modrm(0b11, 0, 1), 0xC1);
        // [base]: mod=00, reg=rax(0), rm=rbp(5)
        assert_eq!(modrm(0b00, 0, 5), 0x05);
    }

    #[test]
    fn test_reg_code_mapping() {
        assert_eq!(reg_code("rax").unwrap().code, 0);
        assert!(!reg_code("rax").unwrap().ext);
        assert_eq!(reg_code("r8").unwrap().code, 0);
        assert!(reg_code("r8").unwrap().ext);
        assert_eq!(reg_code("rcx").unwrap().code, 1);
    }

    #[test]
    fn test_encode_ret() {
        let mut enc = Encoder::new();
        enc.encode_ret();
        assert_eq!(enc.buf, vec![0xC3]);
    }

    #[test]
    fn test_encode_push_pop_rbp() {
        let rbp = reg_code("rbp").unwrap();
        let mut enc = Encoder::new();
        enc.encode_push_reg(rbp);
        enc.encode_pop_reg(rbp);
        assert_eq!(enc.buf, vec![0x55, 0x5D]); // push rbp, pop rbp
    }

    #[test]
    fn test_encode_mov_reg_reg() {
        let rsp = reg_code("rsp").unwrap();
        let rbp = reg_code("rbp").unwrap();
        let mut enc = Encoder::new();
        enc.encode_mov_reg_reg(rsp, rbp, OpSize::Qword);
        // REX.W + 89 ModRM(11, rsp=4, rbp=5) = mov %rsp, %rbp
        assert_eq!(enc.buf, vec![0x48, 0x89, modrm(0b11, 4, 5)]);
    }

    #[test]
    fn test_encode_sub_imm_small() {
        let rsp = reg_code("rsp").unwrap();
        let mut enc = Encoder::new();
        enc.encode_sub_imm_reg(16, rsp, OpSize::Qword);
        // REX.W + 83 /5 imm8 = sub $16, %rsp
        assert_eq!(enc.buf, vec![0x48, 0x83, modrm(0b11, 5, 4), 0x10]);
    }

    #[test]
    fn test_encode_xor_eax_eax() {
        let rax = reg_code("rax").unwrap();
        let mut enc = Encoder::new();
        enc.encode_xor_reg_reg(rax, rax, OpSize::Dword);
        // 31 C0 = xorl %eax, %eax
        assert_eq!(enc.buf, vec![0x31, modrm(0b11, 0, 0)]);
    }

    #[test]
    fn test_encode_cmp_imm_reg() {
        let eax = reg_code("rax").unwrap();
        let mut enc = Encoder::new();
        enc.encode_cmp_imm_reg(42, eax, OpSize::Dword);
        // 83 F8 2A = cmpl $42, %eax
        assert_eq!(enc.buf, vec![0x83, modrm(0b11, 7, 0), 42]);
    }

    #[test]
    fn test_condcode_negate() {
        assert_eq!(CondCode::E.negate(), CondCode::NE);
        assert_eq!(CondCode::L.negate(), CondCode::GE);
        assert_eq!(CondCode::B.negate(), CondCode::AE);
    }

    #[test]
    fn test_encode_call_rel32() {
        let mut enc = Encoder::new();
        let off = enc.encode_call_rel32();
        assert_eq!(enc.buf[0], 0xE8);
        assert_eq!(off, 1); // rel32 starts at byte 1
    }

    #[test]
    fn test_patch_rel32() {
        let mut enc = Encoder::new();
        let off = enc.encode_jmp_rel32();
        enc.encode_nop();
        enc.encode_nop();
        let target = enc.buf.len();
        enc.patch_rel32(off, target);
        // rel32 should be 2 (target - (off + 4) = 7 - 5 = 2)
        let patched = i32::from_le_bytes([enc.buf[off], enc.buf[off + 1], enc.buf[off + 2], enc.buf[off + 3]]);
        assert_eq!(patched, 2);
    }

    #[test]
    fn test_encode_push_r12() {
        let r12 = reg_code("r12").unwrap();
        let mut enc = Encoder::new();
        enc.encode_push_reg(r12);
        // REX.B + 50+4 = push r12
        assert_eq!(enc.buf, vec![0x41, 0x54]);
    }
}
