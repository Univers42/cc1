// backend/native/x86_64/assembler.rs — Builtin assembler for x86-64.
//
// Assembles AT&T syntax x86-64 assembly text into an ELF relocatable object
// file using the ElfWriter. This avoids the need for an external `as` tool.

#![allow(dead_code)]

use crate::backend::native::elf::types::*;
use crate::backend::native::elf::writer::{ElfWriter, Relocation, Symbol};
use crate::backend::native::x86_64::encoding::*;
use std::collections::HashMap;

/// Assembled result of one section.
struct AsmSection {
    /// Section index in ElfWriter.
    idx: usize,
    /// Machine code bytes.
    code: Vec<u8>,
    /// Relocations pending for this section.
    relocs: Vec<PendingReloc>,
}

/// A relocation that needs to be resolved after assembly.
struct PendingReloc {
    /// Offset into the section's code buffer where the rel32 lives.
    offset: usize,
    /// Symbol name being referenced.
    symbol: String,
    /// Relocation type.
    rtype: u32,
    /// Addend.
    addend: i64,
}

/// Label state — either resolved (known offset) or forward-referenced.
struct LabelInfo {
    /// Offset within the current section, if resolved.
    offset: Option<usize>,
    /// Locations (section, code-offset) needing backpatching when resolved.
    fixups: Vec<(usize, usize)>,
}

/// The assembler.
pub struct X86_64Assembler {
    writer: ElfWriter,
    /// Current section name → AsmSection.
    sections: HashMap<String, AsmSection>,
    /// Active section name.
    current_section: String,
    /// Symbol table: name → (section name, offset, is_global).
    symbols: HashMap<String, SymbolDef>,
    /// Labels within current function / section.
    labels: HashMap<String, LabelInfo>,
    /// Encoder for emitting bytes.
    encoder: Encoder,
}

struct SymbolDef {
    section: String,
    offset: usize,
    size: usize,
    is_global: bool,
    is_function: bool,
}

impl X86_64Assembler {
    pub fn new() -> Self {
        let writer = ElfWriter::new(EM_X86_64);
        Self {
            writer,
            sections: HashMap::new(),
            current_section: ".text".into(),
            symbols: HashMap::new(),
            labels: HashMap::new(),
            encoder: Encoder::new(),
        }
    }

    /// Assemble a complete AT&T syntax assembly text into an ELF .o file.
    pub fn assemble(&mut self, asm_text: &str) -> Vec<u8> {
        // Ensure .text section exists
        self.ensure_section(".text", SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR, 16);

        for line in asm_text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue; // skip blank lines and comments
            }
            self.assemble_line(line);
        }

        self.finalize()
    }

    fn ensure_section(&mut self, name: &str, shtype: u32, flags: u64, align: u64) {
        if !self.sections.contains_key(name) {
            let idx = self.writer.add_section(name, shtype, flags, align);
            self.sections.insert(name.into(), AsmSection {
                idx,
                code: Vec::new(),
                relocs: Vec::new(),
            });
        }
    }

    fn current_code(&mut self) -> &mut Vec<u8> {
        let sec = self.sections.get_mut(&self.current_section).unwrap();
        &mut sec.code
    }

    fn current_offset(&self) -> usize {
        self.sections.get(&self.current_section).map_or(0, |s| s.code.len())
    }

    fn emit_bytes_to_section(&mut self, bytes: &[u8]) {
        let sec = self.sections.get_mut(&self.current_section).unwrap();
        sec.code.extend_from_slice(bytes);
    }

    fn assemble_line(&mut self, line: &str) {
        // Check for label definitions (name followed by colon)
        if let Some(label) = line.strip_suffix(':') {
            let label = label.trim();
            self.define_label(label);
            return;
        }

        // Check for directives (start with '.')
        if line.starts_with('.') {
            self.handle_directive(line);
            return;
        }

        // Otherwise it's an instruction
        self.assemble_instruction(line);
    }

    fn define_label(&mut self, name: &str) {
        let offset = self.current_offset();
        let section = self.current_section.clone();

        // Update label info
        if let Some(info) = self.labels.get(name) {
            if info.offset.is_none() {
                // Collect fixup data before mutating
                let fixups: Vec<(usize, usize)> = info.fixups.clone();
                let section = self.current_section.clone();
                let code = self.sections.get_mut(&section).unwrap();
                for &(_, patch_offset) in &fixups {
                    let rel = (offset as i64) - ((patch_offset + 4) as i64);
                    let rel32 = rel as i32;
                    code.code[patch_offset..patch_offset + 4].copy_from_slice(&rel32.to_le_bytes());
                }
                let label_info = self.labels.get_mut(name).unwrap();
                label_info.offset = Some(offset);
                label_info.fixups.clear();
            }
        } else {
            self.labels.insert(name.into(), LabelInfo {
                offset: Some(offset),
                fixups: Vec::new(),
            });
        }

        // Is it a global/function symbol?
        if !name.starts_with('.') && !name.starts_with("_") {
            // Only add to symbol table if not already there
            if !self.symbols.contains_key(name) {
                self.symbols.insert(name.into(), SymbolDef {
                    section: section,
                    offset,
                    size: 0,
                    is_global: false,
                    is_function: false,
                });
            } else {
                let sym = self.symbols.get_mut(name).unwrap();
                sym.offset = offset;
                sym.section = self.current_section.clone();
            }
        }
    }

    fn handle_directive(&mut self, line: &str) {
        let parts: Vec<&str> = line.splitn(2, |c: char| c.is_whitespace() || c == '\t').collect();
        let directive = parts[0];
        let args = if parts.len() > 1 { parts[1].trim() } else { "" };

        match directive {
            ".text" => {
                self.ensure_section(".text", SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR, 16);
                self.current_section = ".text".into();
            }
            ".data" => {
                self.ensure_section(".data", SHT_PROGBITS, SHF_ALLOC | SHF_WRITE, 8);
                self.current_section = ".data".into();
            }
            ".bss" => {
                self.ensure_section(".bss", SHT_NOBITS, SHF_ALLOC | SHF_WRITE, 8);
                self.current_section = ".bss".into();
            }
            ".section" => {
                // .section .rodata or .section .note.GNU-stack,"",@progbits
                let sec_name = args.split(',').next().unwrap_or("").trim();
                if sec_name.contains("rodata") {
                    self.ensure_section(".rodata", SHT_PROGBITS, SHF_ALLOC, 8);
                    self.current_section = ".rodata".into();
                } else if sec_name.contains("note.GNU-stack") {
                    self.ensure_section(".note.GNU-stack", SHT_PROGBITS, 0, 1);
                    self.current_section = ".note.GNU-stack".into();
                } else {
                    self.ensure_section(sec_name, SHT_PROGBITS, SHF_ALLOC, 1);
                    self.current_section = sec_name.to_string();
                }
            }
            ".globl" => {
                let name = args.trim();
                if let Some(sym) = self.symbols.get_mut(name) {
                    sym.is_global = true;
                } else {
                    self.symbols.insert(name.into(), SymbolDef {
                        section: self.current_section.clone(),
                        offset: 0,
                        size: 0,
                        is_global: true,
                        is_function: false,
                    });
                }
            }
            ".type" => {
                // .type name, @function or @object
                let parts: Vec<&str> = args.splitn(2, ',').collect();
                if parts.len() == 2 {
                    let name = parts[0].trim();
                    let ty = parts[1].trim();
                    if ty == "@function" {
                        if let Some(sym) = self.symbols.get_mut(name) {
                            sym.is_function = true;
                        }
                    }
                }
            }
            ".size" => {
                // .size name, .-name → compute size
                let parts: Vec<&str> = args.splitn(2, ',').collect();
                if parts.len() == 2 {
                    let name = parts[0].trim();
                    let _expr = parts[1].trim();
                    let cur = self.current_offset();
                    if let Some(sym) = self.symbols.get_mut(name) {
                        sym.size = cur.saturating_sub(sym.offset);
                    }
                }
            }
            ".align" => {
                if let Ok(align) = args.trim().parse::<usize>() {
                    let offset = self.current_offset();
                    let padding = (align - (offset % align)) % align;
                    for _ in 0..padding {
                        self.emit_bytes_to_section(&[0x90]); // NOP padding in .text
                    }
                }
            }
            ".byte" => {
                for val_str in args.split(',') {
                    if let Ok(v) = val_str.trim().parse::<u8>() {
                        self.emit_bytes_to_section(&[v]);
                    }
                }
            }
            ".short" | ".value" => {
                for val_str in args.split(',') {
                    if let Ok(v) = val_str.trim().parse::<u16>() {
                        self.emit_bytes_to_section(&v.to_le_bytes());
                    }
                }
            }
            ".long" => {
                for val_str in args.split(',') {
                    let val_str = val_str.trim();
                    // Strip trailing comments
                    let val_str = val_str.split('#').next().unwrap_or("").trim();
                    if let Ok(v) = val_str.parse::<u32>() {
                        self.emit_bytes_to_section(&v.to_le_bytes());
                    }
                }
            }
            ".quad" => {
                for val_str in args.split(',') {
                    let val_str = val_str.trim().split('#').next().unwrap_or("").trim();
                    if let Ok(v) = val_str.parse::<u64>() {
                        self.emit_bytes_to_section(&v.to_le_bytes());
                    } else {
                        // Symbol reference — emit relocation
                        let sec = self.sections.get_mut(&self.current_section).unwrap();
                        let offset = sec.code.len();
                        sec.code.extend_from_slice(&[0u8; 8]);
                        sec.relocs.push(PendingReloc {
                            offset,
                            symbol: val_str.to_string(),
                            rtype: R_X86_64_64,
                            addend: 0,
                        });
                    }
                }
            }
            ".zero" => {
                if let Ok(n) = args.trim().parse::<usize>() {
                    self.emit_bytes_to_section(&vec![0u8; n]);
                }
            }
            ".comm" => {
                // .comm name,size,align
                let parts: Vec<&str> = args.split(',').collect();
                if parts.len() >= 2 {
                    let name = parts[0].trim();
                    let size: usize = parts[1].trim().parse().unwrap_or(0);
                    let _align: u64 = parts.get(2).and_then(|s| s.trim().parse().ok()).unwrap_or(1);
                    self.symbols.insert(name.into(), SymbolDef {
                        section: "*COM*".into(),
                        offset: 0,
                        size,
                        is_global: true,
                        is_function: false,
                    });
                }
            }
            ".file" | ".ident" => {
                // Ignored for now
            }
            _ => {
                // Unknown directive — ignore
            }
        }
    }

    fn assemble_instruction(&mut self, line: &str) {
        // Parse AT&T instruction: mnemonic\toperand, operand
        let line = line.trim();
        let (mnemonic, rest) = match line.find(|c: char| c.is_whitespace() || c == '\t') {
            Some(pos) => (line[..pos].trim(), line[pos..].trim()),
            None => (line, ""),
        };

        let operands: Vec<&str> = if rest.is_empty() {
            vec![]
        } else {
            rest.split(',').map(|s| s.trim()).collect()
        };

        self.encoder.buf.clear();

        match mnemonic {
            "nop" => self.encoder.encode_nop(),
            "syscall" => self.encoder.encode_syscall(),
            "ret" | "retq" => self.encoder.encode_ret(),
            "leave" | "leaveq" => self.encoder.encode_leave(),
            "cltd" | "cdq" => self.encoder.encode_cltd(),
            "cqto" | "cqo" => self.encoder.encode_cqto(),

            "pushq" => {
                if let Some(reg) = parse_register(operands.get(0).unwrap_or(&"")) {
                    self.encoder.encode_push_reg(reg);
                }
            }
            "popq" => {
                if let Some(reg) = parse_register(operands.get(0).unwrap_or(&"")) {
                    self.encoder.encode_pop_reg(reg);
                }
            }

            // MOV family
            m if m.starts_with("mov") => {
                if operands.len() == 2 {
                    let size = mnemonic_size(m, "mov");
                    self.encode_mov(&operands, size);
                }
            }

            // ADD
            m if m.starts_with("add") => {
                if operands.len() == 2 {
                    let size = mnemonic_size(m, "add");
                    self.encode_alu_op(AluOp::Add, &operands, size);
                }
            }

            // SUB
            m if m.starts_with("sub") => {
                if operands.len() == 2 {
                    let size = mnemonic_size(m, "sub");
                    self.encode_alu_op(AluOp::Sub, &operands, size);
                }
            }

            // IMUL
            m if m.starts_with("imul") => {
                if operands.len() == 2 {
                    let size = mnemonic_size(m, "imul");
                    if let (Some(src), Some(dst)) = (parse_register(operands[0]), parse_register(operands[1])) {
                        self.encoder.encode_imul_reg_reg(src, dst, size);
                    }
                }
            }

            // IDIV
            m if m.starts_with("idiv") => {
                if operands.len() == 1 {
                    let size = mnemonic_size(m, "idiv");
                    if let Some(reg) = parse_register(operands[0]) {
                        self.encoder.encode_idiv_reg(reg, size);
                    }
                }
            }

            // NEG
            m if m.starts_with("neg") => {
                if operands.len() == 1 {
                    let size = mnemonic_size(m, "neg");
                    if let Some(reg) = parse_register(operands[0]) {
                        self.encoder.encode_neg_reg(reg, size);
                    }
                }
            }

            // NOT
            m if m.starts_with("not") => {
                if operands.len() == 1 {
                    let size = mnemonic_size(m, "not");
                    if let Some(reg) = parse_register(operands[0]) {
                        self.encoder.encode_not_reg(reg, size);
                    }
                }
            }

            // AND
            m if m.starts_with("and") => {
                if operands.len() == 2 {
                    let size = mnemonic_size(m, "and");
                    if let (Some(src), Some(dst)) = (parse_register(operands[0]), parse_register(operands[1])) {
                        self.encoder.encode_and_reg_reg(src, dst, size);
                    }
                }
            }

            // OR
            m if m.starts_with("or") && !m.starts_with("ord") => {
                if operands.len() == 2 {
                    let size = mnemonic_size(m, "or");
                    if let (Some(src), Some(dst)) = (parse_register(operands[0]), parse_register(operands[1])) {
                        self.encoder.encode_or_reg_reg(src, dst, size);
                    }
                }
            }

            // XOR
            m if m.starts_with("xor") => {
                if operands.len() == 2 {
                    let size = mnemonic_size(m, "xor");
                    if let (Some(src), Some(dst)) = (parse_register(operands[0]), parse_register(operands[1])) {
                        self.encoder.encode_xor_reg_reg(src, dst, size);
                    }
                }
            }

            // Shifts
            m if m.starts_with("shl") => {
                if operands.len() == 2 {
                    let size = mnemonic_size(m, "shl");
                    self.encode_shift(ShiftOp::Shl, &operands, size);
                }
            }
            m if m.starts_with("shr") => {
                if operands.len() == 2 {
                    let size = mnemonic_size(m, "shr");
                    self.encode_shift(ShiftOp::Shr, &operands, size);
                }
            }
            m if m.starts_with("sar") => {
                if operands.len() == 2 {
                    let size = mnemonic_size(m, "sar");
                    self.encode_shift(ShiftOp::Sar, &operands, size);
                }
            }

            // CMP
            m if m.starts_with("cmp") && !m.starts_with("cmpxchg") => {
                if operands.len() == 2 {
                    let size = mnemonic_size(m, "cmp");
                    self.encode_cmp(&operands, size);
                }
            }

            // TEST
            m if m.starts_with("test") => {
                if operands.len() == 2 {
                    let size = mnemonic_size(m, "test");
                    if let (Some(src), Some(dst)) = (parse_register(operands[0]), parse_register(operands[1])) {
                        self.encoder.encode_test_reg_reg(src, dst, size);
                    }
                }
            }

            // SETcc
            m if m.starts_with("set") => {
                if operands.len() == 1 {
                    if let (Some(cc), Some(dst)) = (parse_condcode(&m[3..]), parse_register(operands[0])) {
                        self.encoder.encode_setcc(cc, dst);
                    }
                }
            }

            // LEA
            m if m.starts_with("lea") => {
                if operands.len() == 2 {
                    let size = mnemonic_size(m, "lea");
                    // leaq disp(%base), %dst
                    if let (Some((base, disp)), Some(dst)) = (parse_memory(operands[0]), parse_register(operands[1])) {
                        self.encoder.encode_lea(base, disp, dst, size);
                    }
                }
            }

            // JMP
            "jmp" => {
                if operands.len() == 1 {
                    self.encode_jump(operands[0]);
                }
            }

            // Jcc
            m if m.starts_with('j') && m.len() > 1 => {
                if operands.len() == 1 {
                    if let Some(cc) = parse_condcode(&m[1..]) {
                        self.encode_jcc(cc, operands[0]);
                    }
                }
            }

            // CALL
            "call" => {
                if operands.len() == 1 {
                    let target = operands[0].trim();
                    if target.starts_with('*') {
                        // Indirect call
                        let reg_name = target[1..].trim();
                        if let Some(rc) = parse_register(reg_name) {
                            self.encoder.encode_call_indirect(rc);
                        }
                    } else {
                        self.encode_call(target);
                    }
                }
            }

            // MOVZX
            m if m.starts_with("movz") => {
                if operands.len() == 2 {
                    if let (Some(src), Some(dst)) = (parse_register(operands[0]), parse_register(operands[1])) {
                        let dst_size = if m.ends_with('q') || m.contains("bq") || m.contains("wq") {
                            OpSize::Qword
                        } else {
                            OpSize::Dword
                        };
                        self.encoder.encode_movzx_byte(src, dst, dst_size);
                    }
                }
            }

            // MOVSX
            m if m.starts_with("movs") && !m.starts_with("movss") => {
                if operands.len() == 2 {
                    if let (Some(src), Some(dst)) = (parse_register(operands[0]), parse_register(operands[1])) {
                        if m.contains("lq") || m == "movsxd" {
                            self.encoder.encode_movsxd(src, dst);
                        } else {
                            let dst_size = if m.ends_with('q') { OpSize::Qword } else { OpSize::Dword };
                            self.encoder.encode_movsx_byte(src, dst, dst_size);
                        }
                    }
                }
            }

            // CMOVcc
            m if m.starts_with("cmov") => {
                // cmovne, cmove, etc.
                // For now just encode as MOV (simplified)
                if operands.len() == 2 {
                    // TODO: proper CMOVcc encoding
                }
            }

            _ => {
                // Unknown mnemonic — emit as comment byte for debugging
            }
        }

        let bytes = self.encoder.buf.clone();
        self.emit_bytes_to_section(&bytes);
    }

    // Encoding helpers

    fn encode_mov(&mut self, operands: &[&str], size: OpSize) {
        let src = operands[0].trim();
        let dst = operands[1].trim();

        if let Some(imm) = parse_immediate(src) {
            // MOV $imm, reg
            if let Some(dst_reg) = parse_register(dst) {
                if size == OpSize::Qword && (imm > i32::MAX as i64 || imm < i32::MIN as i64) {
                    self.encoder.encode_mov_imm64_reg(imm, dst_reg);
                } else {
                    self.encoder.encode_mov_imm32_reg(imm as i32, dst_reg);
                }
            } else if let Some((base, disp)) = parse_memory(dst) {
                // MOV $imm, mem — need intermediate encoding
                // For simplicity, first mov to temp then store
                // Real assembler would use C7 opcode directly
                self.encoder.encode_mov_imm32_reg(imm as i32, RegCode::new(0, false)); // rax
                self.encoder.encode_mov_reg_mem(RegCode::new(0, false), base, disp, size);
            }
        } else if let Some(src_reg) = parse_register(src) {
            if let Some(dst_reg) = parse_register(dst) {
                // MOV reg, reg
                self.encoder.encode_mov_reg_reg(src_reg, dst_reg, size);
            } else if let Some((base, disp)) = parse_memory(dst) {
                // MOV reg, mem
                self.encoder.encode_mov_reg_mem(src_reg, base, disp, size);
            }
        } else if let Some((base, disp)) = parse_memory(src) {
            if let Some(dst_reg) = parse_register(dst) {
                // MOV mem, reg
                self.encoder.encode_mov_mem_reg(base, disp, dst_reg, size);
            }
        } else if src.contains("(%rip)") {
            // RIP-relative addressing — emit relocation
            if let Some(dst_reg) = parse_register(dst) {
                let sym = src.split('(').next().unwrap_or("").trim();
                let offset = self.current_offset() + self.encoder.buf.len();
                // LEA sym(%rip), dst  (will be patched by linker)
                self.encoder.encode_lea(RegCode::new(5, false), 0, dst_reg, size);
                let sec = &self.current_section;
                if let Some(section) = self.sections.get_mut(sec.as_str()) {
                    section.relocs.push(PendingReloc {
                        offset: offset + self.encoder.buf.len() - 4, // point at the disp32
                        symbol: sym.to_string(),
                        rtype: R_X86_64_PC32,
                        addend: -4,
                    });
                }
            }
        }
    }

    fn encode_alu_op(&mut self, op: AluOp, operands: &[&str], size: OpSize) {
        let src = operands[0].trim();
        let dst = operands[1].trim();

        if let Some(imm) = parse_immediate(src) {
            if let Some(dst_reg) = parse_register(dst) {
                match op {
                    AluOp::Add => self.encoder.encode_add_imm_reg(imm as i32, dst_reg, size),
                    AluOp::Sub => self.encoder.encode_sub_imm_reg(imm as i32, dst_reg, size),
                }
            }
        } else if let Some(src_reg) = parse_register(src) {
            if let Some(dst_reg) = parse_register(dst) {
                match op {
                    AluOp::Add => self.encoder.encode_add_reg_reg(src_reg, dst_reg, size),
                    AluOp::Sub => self.encoder.encode_sub_reg_reg(src_reg, dst_reg, size),
                }
            }
        }
    }

    fn encode_cmp(&mut self, operands: &[&str], size: OpSize) {
        let src = operands[0].trim();
        let dst = operands[1].trim();

        if let Some(imm) = parse_immediate(src) {
            if let Some(dst_reg) = parse_register(dst) {
                self.encoder.encode_cmp_imm_reg(imm as i32, dst_reg, size);
            }
        } else if let Some(src_reg) = parse_register(src) {
            if let Some(dst_reg) = parse_register(dst) {
                self.encoder.encode_cmp_reg_reg(src_reg, dst_reg, size);
            }
        }
    }

    fn encode_shift(&mut self, op: ShiftOp, operands: &[&str], size: OpSize) {
        let amount = operands[0].trim();
        let dst = operands[1].trim();

        if let Some(dst_reg) = parse_register(dst) {
            if let Some(imm) = parse_immediate(amount) {
                match op {
                    ShiftOp::Shl => self.encoder.encode_shl_imm(imm as u8, dst_reg, size),
                    ShiftOp::Shr => self.encoder.encode_shr_imm(imm as u8, dst_reg, size),
                    ShiftOp::Sar => self.encoder.encode_shr_imm(imm as u8, dst_reg, size), // TODO: sar_imm
                }
            } else if amount == "%cl" {
                match op {
                    ShiftOp::Shl => self.encoder.encode_shl_cl(dst_reg, size),
                    ShiftOp::Shr => self.encoder.encode_shr_cl(dst_reg, size),
                    ShiftOp::Sar => self.encoder.encode_sar_cl(dst_reg, size),
                }
            }
        }
    }

    fn encode_jump(&mut self, target: &str) {
        let target = target.trim();
        if let Some(info) = self.labels.get(target) {
            if let Some(label_offset) = info.offset {
                // Backward jump — known target
                let jmp_start = self.current_offset();
                let patch_offset = self.encoder.encode_jmp_rel32();
                // Compute relative offset: target - (current + instruction_length)
                let rel = (label_offset as i64) - ((jmp_start + self.encoder.buf.len()) as i64);
                let rel32 = rel as i32;
                self.encoder.buf[patch_offset..patch_offset + 4].copy_from_slice(&rel32.to_le_bytes());
            } else {
                // Forward jump — add fixup
                let code_offset = self.current_offset();
                let patch_offset = self.encoder.encode_jmp_rel32();
                let abs_patch = code_offset + patch_offset;
                let labels = &mut self.labels;
                if let Some(info) = labels.get_mut(target) {
                    info.fixups.push((0, abs_patch));
                }
            }
        } else {
            // Unknown label — could be external symbol, emit relocation
            let code_offset = self.current_offset();
            let patch_offset = self.encoder.encode_jmp_rel32();
            let abs_patch = code_offset + patch_offset;
            self.labels.insert(target.to_string(), LabelInfo {
                offset: None,
                fixups: vec![(0, abs_patch)],
            });
        }
    }

    fn encode_jcc(&mut self, cc: CondCode, target: &str) {
        let target = target.trim();
        if let Some(info) = self.labels.get(target) {
            if let Some(label_offset) = info.offset {
                let jmp_start = self.current_offset();
                let patch_offset = self.encoder.encode_jcc_rel32(cc);
                let rel = (label_offset as i64) - ((jmp_start + self.encoder.buf.len()) as i64);
                let rel32 = rel as i32;
                self.encoder.buf[patch_offset..patch_offset + 4].copy_from_slice(&rel32.to_le_bytes());
            } else {
                let code_offset = self.current_offset();
                let patch_offset = self.encoder.encode_jcc_rel32(cc);
                let abs_patch = code_offset + patch_offset;
                let labels = &mut self.labels;
                if let Some(info) = labels.get_mut(target) {
                    info.fixups.push((0, abs_patch));
                }
            }
        } else {
            let code_offset = self.current_offset();
            let patch_offset = self.encoder.encode_jcc_rel32(cc);
            let abs_patch = code_offset + patch_offset;
            self.labels.insert(target.to_string(), LabelInfo {
                offset: None,
                fixups: vec![(0, abs_patch)],
            });
        }
    }

    fn encode_call(&mut self, target: &str) {
        let target = target.trim();
        let code_offset = self.current_offset();
        let patch_offset = self.encoder.encode_call_rel32();
        let abs_patch = code_offset + patch_offset;

        // Check if it's a known label (already defined)
        if let Some(info) = self.labels.get(target) {
            if let Some(label_offset) = info.offset {
                // Backward call — resolve now
                let rel = (label_offset as i64) - ((code_offset + self.encoder.buf.len()) as i64);
                let rel32 = rel as i32;
                self.encoder.buf[patch_offset..patch_offset + 4].copy_from_slice(&rel32.to_le_bytes());
                return;
            } else {
                // Forward call — label seen but not yet defined, add fixup
                let labels = &mut self.labels;
                if let Some(info) = labels.get_mut(target) {
                    info.fixups.push((0, abs_patch));
                }
                return;
            }
        }

        // Label not yet encountered — create forward reference entry
        // (will be backpatched when the label is defined)
        self.labels.insert(target.to_string(), LabelInfo {
            offset: None,
            fixups: vec![(0, abs_patch)],
        });
    }

    fn finalize(&mut self) -> Vec<u8> {
        // Copy section code into ElfWriter
        let section_names: Vec<String> = self.sections.keys().cloned().collect();
        for name in &section_names {
            let sec = &self.sections[name];
            let idx = sec.idx;
            *self.writer.section_data(idx) = sec.code.clone();
        }

        // Add symbols
        for (name, sym) in &self.symbols {
            let sec_idx = if sym.section == "*COM*" {
                SHN_COMMON as usize
            } else {
                self.sections.get(&sym.section).map_or(0, |s| s.idx)
            };
            let bind = if sym.is_global { STB_GLOBAL } else { STB_LOCAL };
            let stype = if sym.is_function { STT_FUNC } else { STT_OBJECT };

            self.writer.add_symbol(Symbol {
                name: name.clone(),
                value: sym.offset as u64,
                size: sym.size as u64,
                binding: bind,
                sym_type: stype,
                section_idx: sec_idx as u16,
            });
        }

        // Add relocations
        for name in &section_names {
            let sec = &self.sections[name];
            for reloc in &sec.relocs {
                // Find or create symbol index  
                self.writer.add_relocation(Relocation {
                    section_name: name.clone(),
                    offset: reloc.offset as u64,
                    symbol: reloc.symbol.clone(),
                    rel_type: reloc.rtype,
                    addend: reloc.addend,
                });
            }
        }

        self.writer.finalize()
    }
}

// ── Parsing helpers ───────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum AluOp { Add, Sub }

#[derive(Clone, Copy)]
enum ShiftOp { Shl, Shr, Sar }

/// Parse an AT&T register operand like %rax → RegCode.
fn parse_register(s: &str) -> Option<RegCode> {
    let s = s.trim();
    let name = s.strip_prefix('%')?;
    reg_code(name)
}

/// Parse an immediate operand like $42 → i64.
fn parse_immediate(s: &str) -> Option<i64> {
    let s = s.trim();
    let num_str = s.strip_prefix('$')?;
    if num_str.starts_with("0x") || num_str.starts_with("0X") {
        i64::from_str_radix(&num_str[2..], 16).ok()
    } else {
        num_str.parse::<i64>().ok()
    }
}

/// Parse a memory operand like -8(%rbp) → (RegCode for base, displacement).
fn parse_memory(s: &str) -> Option<(RegCode, i32)> {
    let s = s.trim();
    // Format: disp(%base) or (%base)
    if let Some(paren_start) = s.find('(') {
        let disp_str = &s[..paren_start];
        let inside = s[paren_start + 1..].strip_suffix(')')?;
        let base = parse_register(inside)?;
        let disp = if disp_str.is_empty() {
            0
        } else {
            disp_str.parse::<i32>().ok()?
        };
        Some((base, disp))
    } else {
        None
    }
}

/// Determine operand size from mnemonic suffix.
fn mnemonic_size(full: &str, base: &str) -> OpSize {
    let suffix = &full[base.len()..];
    match suffix {
        "b" => OpSize::Byte,
        "w" => OpSize::Word,
        "l" => OpSize::Dword,
        "q" => OpSize::Qword,
        _ => OpSize::Qword, // default to 64-bit
    }
}

/// Parse condition code suffix (e.g., "ne", "e", "l", "ge").
fn parse_condcode(s: &str) -> Option<CondCode> {
    match s {
        "o" => Some(CondCode::O),
        "no" => Some(CondCode::NO),
        "b" | "c" | "nae" => Some(CondCode::B),
        "ae" | "nc" | "nb" => Some(CondCode::AE),
        "e" | "z" => Some(CondCode::E),
        "ne" | "nz" => Some(CondCode::NE),
        "be" | "na" => Some(CondCode::BE),
        "a" | "nbe" => Some(CondCode::A),
        "s" => Some(CondCode::S),
        "ns" => Some(CondCode::NS),
        "p" | "pe" => Some(CondCode::P),
        "np" | "po" => Some(CondCode::NP),
        "l" | "nge" => Some(CondCode::L),
        "ge" | "nl" => Some(CondCode::GE),
        "le" | "ng" => Some(CondCode::LE),
        "g" | "nle" => Some(CondCode::G),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_register() {
        let rc = parse_register("%rax").unwrap();
        assert_eq!(rc.code, 0);
        assert!(!rc.ext);

        let rc = parse_register("%r12").unwrap();
        assert_eq!(rc.code, 4);
        assert!(rc.ext);

        assert!(parse_register("$42").is_none());
    }

    #[test]
    fn test_parse_immediate() {
        assert_eq!(parse_immediate("$42"), Some(42));
        assert_eq!(parse_immediate("$-8"), Some(-8));
        assert_eq!(parse_immediate("$0xFF"), Some(255));
        assert!(parse_immediate("%rax").is_none());
    }

    #[test]
    fn test_parse_memory() {
        let (base, disp) = parse_memory("-8(%rbp)").unwrap();
        assert_eq!(base.code, 5); // rbp
        assert_eq!(disp, -8);

        let (base, disp) = parse_memory("(%rax)").unwrap();
        assert_eq!(base.code, 0); // rax
        assert_eq!(disp, 0);
    }

    #[test]
    fn test_mnemonic_size() {
        assert_eq!(mnemonic_size("movq", "mov"), OpSize::Qword);
        assert_eq!(mnemonic_size("movl", "mov"), OpSize::Dword);
        assert_eq!(mnemonic_size("movb", "mov"), OpSize::Byte);
        assert_eq!(mnemonic_size("addw", "add"), OpSize::Word);
    }

    #[test]
    fn test_parse_condcode() {
        assert_eq!(parse_condcode("e"), Some(CondCode::E));
        assert_eq!(parse_condcode("ne"), Some(CondCode::NE));
        assert_eq!(parse_condcode("l"), Some(CondCode::L));
        assert_eq!(parse_condcode("ge"), Some(CondCode::GE));
        assert!(parse_condcode("xyz").is_none());
    }

    #[test]
    fn test_assemble_simple_ret() {
        let mut asm = X86_64Assembler::new();
        let elf = asm.assemble("\t.text\n\t.globl\tmain\nmain:\n\tret\n");

        // Should produce a valid ELF file (starts with magic)
        assert!(elf.len() > 16);
        assert_eq!(&elf[0..4], &[0x7F, b'E', b'L', b'F']);
    }

    #[test]
    fn test_assemble_push_leave_ret() {
        let mut asm = X86_64Assembler::new();
        let elf = asm.assemble(
            "\t.text\n\t.globl\tmain\nmain:\n\tpushq\t%rbp\n\tmovq\t%rsp, %rbp\n\tleave\n\tret\n"
        );

        assert!(elf.len() > 16);
        assert_eq!(&elf[0..4], &[0x7F, b'E', b'L', b'F']);

        // Find the .text section and verify machine code
        // push rbp = 55, mov rsp,rbp = 48 89 e5, leave = c9, ret = c3
        // The machine code should be somewhere in the ELF file
        let expected = [0x55, 0x48, 0x89, 0xE5, 0xC9, 0xC3];
        let found = elf.windows(expected.len()).any(|w| w == expected);
        assert!(found, "expected machine code sequence not found in ELF");
    }
}
