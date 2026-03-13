// backend/native/linker.rs — Builtin static linker.
//
// Takes one or more ELF relocatable object files (as byte buffers) and
// produces a statically-linked ELF executable. Supports a minimal subset
// of linking: symbol resolution, relocation application, and PT_LOAD
// segment creation.

#![allow(dead_code)]

use crate::backend::native::elf::types::*;
use crate::target::Target;
use std::collections::HashMap;

/// A parsed section from an object file.
struct ObjSection {
    name: String,
    shtype: u32,
    flags: u64,
    data: Vec<u8>,
    align: u64,
    /// Relocations targeting this section.
    relocs: Vec<ObjReloc>,
}

/// A relocation entry from an object file.
struct ObjReloc {
    offset: u64,
    rtype: u32,
    sym_name: String,
    addend: i64,
}

/// A symbol from an object file.
struct ObjSymbol {
    name: String,
    section_idx: u16,
    value: u64,
    size: u64,
    binding: u8,
    sym_type: u8,
}

/// Link one or more ELF object files into an ELF executable.
///
/// Returns the raw bytes of the executable.
pub fn link(objects: &[Vec<u8>], target: Target) -> Vec<u8> {
    let mut linker = Linker::new(target);
    for obj in objects {
        linker.add_object(obj);
    }
    linker.link()
}

struct Linker {
    target: Target,
    /// All sections collected from object files.
    sections: Vec<ObjSection>,
    /// Global symbol table: name → (section index in self.sections, offset).
    symbols: HashMap<String, SymbolEntry>,
}

struct SymbolEntry {
    /// Index into self.sections.
    section_idx: usize,
    /// Offset within that section.
    offset: u64,
    /// Symbol size.
    size: u64,
    /// Whether this symbol is a function.
    is_func: bool,
}

impl Linker {
    fn new(target: Target) -> Self {
        Self {
            target,
            sections: Vec::new(),
            symbols: HashMap::new(),
        }
    }

    /// Parse and collect sections/symbols/relocs from one ELF .o file.
    fn add_object(&mut self, data: &[u8]) {
        if data.len() < 64 {
            return; // Too small to be valid ELF
        }

        // Verify magic
        if &data[0..4] != b"\x7fELF" {
            return;
        }

        // Parse ELF header
        let e_shoff = read_u64(data, 40) as usize;
        let e_shentsize = read_u16(data, 58) as usize;
        let e_shnum = read_u16(data, 60) as usize;
        let e_shstrndx = read_u16(data, 62) as usize;

        if e_shoff == 0 || e_shnum == 0 {
            return;
        }

        // Read section headers
        let mut shdrs: Vec<Elf64Shdr> = Vec::new();
        for i in 0..e_shnum {
            let off = e_shoff + i * e_shentsize;
            if off + e_shentsize > data.len() {
                break;
            }
            shdrs.push(parse_shdr(data, off));
        }

        // Read section name string table
        let shstrtab = if e_shstrndx < shdrs.len() {
            let sh = &shdrs[e_shstrndx];
            let start = sh.sh_offset as usize;
            let end = start + sh.sh_size as usize;
            if end <= data.len() {
                &data[start..end]
            } else {
                &[]
            }
        } else {
            &[]
        };

        // Find symtab and strtab
        let mut symtab_idx = None;
        let mut strtab_idx = None;
        for (i, sh) in shdrs.iter().enumerate() {
            let name = read_cstr(shstrtab, sh.sh_name as usize);
            if sh.sh_type == SHT_SYMTAB {
                symtab_idx = Some(i);
                strtab_idx = Some(sh.sh_link as usize);
            }
            let _ = name;
        }

        // Parse symbols
        let mut obj_syms: Vec<ObjSymbol> = Vec::new();
        if let (Some(si), Some(sti)) = (symtab_idx, strtab_idx) {
            let sym_sh = &shdrs[si];
            let str_sh = &shdrs[sti];
            let strtab_start = str_sh.sh_offset as usize;
            let strtab_end = strtab_start + str_sh.sh_size as usize;
            let strtab = if strtab_end <= data.len() {
                &data[strtab_start..strtab_end]
            } else {
                &[]
            };

            let sym_start = sym_sh.sh_offset as usize;
            let sym_count = if sym_sh.sh_entsize > 0 {
                sym_sh.sh_size / sym_sh.sh_entsize
            } else {
                0
            };

            for i in 0..sym_count as usize {
                let off = sym_start + i * 24; // sizeof(Elf64_Sym) = 24
                if off + 24 > data.len() {
                    break;
                }
                let st_name = read_u32(data, off) as usize;
                let st_info = data[off + 4];
                let st_shndx = read_u16(data, off + 6);
                let st_value = read_u64(data, off + 8);
                let st_size = read_u64(data, off + 16);

                let name = read_cstr(strtab, st_name);
                let binding = st_info >> 4;
                let sym_type = st_info & 0xf;

                obj_syms.push(ObjSymbol {
                    name: name.to_string(),
                    section_idx: st_shndx,
                    value: st_value,
                    size: st_size,
                    binding,
                    sym_type,
                });
            }
        }

        // Collect sections and build section index mapping
        let _base_section_idx = self.sections.len();
        let mut section_map: HashMap<usize, usize> = HashMap::new(); // obj_shdr_idx → self.sections idx

        for (i, sh) in shdrs.iter().enumerate() {
            let name = read_cstr(shstrtab, sh.sh_name as usize).to_string();
            if sh.sh_type == SHT_PROGBITS || sh.sh_type == SHT_NOBITS {
                if name == ".note.GNU-stack" || name.is_empty() {
                    continue;
                }
                let sec_data = if sh.sh_type == SHT_NOBITS {
                    vec![0u8; sh.sh_size as usize]
                } else {
                    let start = sh.sh_offset as usize;
                    let end = start + sh.sh_size as usize;
                    if end <= data.len() {
                        data[start..end].to_vec()
                    } else {
                        Vec::new()
                    }
                };

                let idx = self.sections.len();
                section_map.insert(i, idx);
                self.sections.push(ObjSection {
                    name,
                    shtype: sh.sh_type,
                    flags: sh.sh_flags,
                    data: sec_data,
                    align: sh.sh_addralign,
                    relocs: Vec::new(),
                });
            }
        }

        // Parse relocations
        for (_i, sh) in shdrs.iter().enumerate() {
            if sh.sh_type == SHT_RELA {
                let target_shdr_idx = sh.sh_info as usize;
                if let Some(&target_sec_idx) = section_map.get(&target_shdr_idx) {
                    let rela_start = sh.sh_offset as usize;
                    let rela_count = if sh.sh_entsize > 0 {
                        sh.sh_size / sh.sh_entsize
                    } else {
                        0
                    };

                    for j in 0..rela_count as usize {
                        let off = rela_start + j * 24; // sizeof(Elf64_Rela) = 24
                        if off + 24 > data.len() {
                            break;
                        }
                        let r_offset = read_u64(data, off);
                        let r_info = read_u64(data, off + 8);
                        let r_addend = read_i64(data, off + 16);
                        let sym_idx = (r_info >> 32) as usize;
                        let rtype = (r_info & 0xffffffff) as u32;

                        let sym_name = if sym_idx < obj_syms.len() {
                            obj_syms[sym_idx].name.clone()
                        } else {
                            String::new()
                        };

                        self.sections[target_sec_idx].relocs.push(ObjReloc {
                            offset: r_offset,
                            rtype,
                            sym_name,
                            addend: r_addend,
                        });
                    }
                }
            }
        }

        // Register global symbols
        for sym in &obj_syms {
            if sym.binding == STB_GLOBAL && !sym.name.is_empty() && sym.section_idx != SHN_UNDEF {
                if let Some(&sec_idx) = section_map.get(&(sym.section_idx as usize)) {
                    self.symbols.insert(sym.name.clone(), SymbolEntry {
                        section_idx: sec_idx,
                        offset: sym.value,
                        size: sym.size,
                        is_func: sym.sym_type == STT_FUNC,
                    });
                }
            }
        }
    }

    /// Link all collected objects into an ELF executable.
    fn link(&mut self) -> Vec<u8> {
        // Layout: ELF header + Program headers + sections (text, rodata, data, bss)
        // We use a simple layout:
        //   Virtual base address: 0x400000 (standard for x86-64 static executables)
        //   Segment 1 (RX): .text
        //   Segment 2 (R): .rodata
        //   Segment 3 (RW): .data, .bss

        let base_addr: u64 = 0x400000;
        let page_size: u64 = 0x1000;

        // Categorize sections
        let mut text_sections: Vec<usize> = Vec::new();
        let mut rodata_sections: Vec<usize> = Vec::new();
        let mut data_sections: Vec<usize> = Vec::new();

        for (i, sec) in self.sections.iter().enumerate() {
            if sec.flags & SHF_EXECINSTR != 0 {
                text_sections.push(i);
            } else if sec.flags & SHF_WRITE != 0 {
                data_sections.push(i);
            } else if sec.flags & SHF_ALLOC != 0 {
                rodata_sections.push(i);
            }
        }

        // Compute section offsets within each segment
        // File layout: ELF header (64) + 3 Phdrs (3*56=168) = 232 bytes header
        let ehdr_size = 64u64;
        let phdr_size = 56u64;
        let num_phdrs = 3u64;
        let headers_size = ehdr_size + num_phdrs * phdr_size;
        let first_section_offset = align_up(headers_size, page_size);

        // Assign virtual addresses and file offsets to each section
        let mut section_vaddrs: Vec<u64> = vec![0; self.sections.len()];
        let mut section_offsets: Vec<u64> = vec![0; self.sections.len()];

        let mut current_offset = first_section_offset;
        let mut current_vaddr = base_addr + first_section_offset;

        // Text segment
        let text_seg_offset = current_offset;
        let text_seg_vaddr = current_vaddr;
        let mut text_seg_size = 0u64;
        for &idx in &text_sections {
            let align = self.sections[idx].align.max(1);
            current_offset = align_up(current_offset, align);
            current_vaddr = align_up(current_vaddr, align);
            section_offsets[idx] = current_offset;
            section_vaddrs[idx] = current_vaddr;
            let size = self.sections[idx].data.len() as u64;
            current_offset += size;
            current_vaddr += size;
            text_seg_size = current_offset - text_seg_offset;
        }

        // Align to page for next segment
        current_offset = align_up(current_offset, page_size);
        current_vaddr = align_up(current_vaddr, page_size);

        // Rodata segment
        let rodata_seg_offset = current_offset;
        let rodata_seg_vaddr = current_vaddr;
        let mut rodata_seg_size = 0u64;
        for &idx in &rodata_sections {
            let align = self.sections[idx].align.max(1);
            current_offset = align_up(current_offset, align);
            current_vaddr = align_up(current_vaddr, align);
            section_offsets[idx] = current_offset;
            section_vaddrs[idx] = current_vaddr;
            let size = self.sections[idx].data.len() as u64;
            current_offset += size;
            current_vaddr += size;
            rodata_seg_size = current_offset - rodata_seg_offset;
        }

        // Align to page for next segment
        current_offset = align_up(current_offset, page_size);
        current_vaddr = align_up(current_vaddr, page_size);

        // Data segment
        let data_seg_offset = current_offset;
        let data_seg_vaddr = current_vaddr;
        let mut data_seg_size = 0u64;
        for &idx in &data_sections {
            let align = self.sections[idx].align.max(1);
            current_offset = align_up(current_offset, align);
            current_vaddr = align_up(current_vaddr, align);
            section_offsets[idx] = current_offset;
            section_vaddrs[idx] = current_vaddr;
            let size = self.sections[idx].data.len() as u64;
            current_offset += size;
            current_vaddr += size;
            data_seg_size = current_offset - data_seg_offset;
        }

        let total_file_size = current_offset;

        // Apply relocations
        for i in 0..self.sections.len() {
            let relocs: Vec<ObjReloc> = std::mem::take(&mut self.sections[i].relocs);
            for reloc in &relocs {
                if let Some(sym) = self.symbols.get(&reloc.sym_name) {
                    let sym_vaddr = section_vaddrs[sym.section_idx] + sym.offset;
                    let reloc_vaddr = section_vaddrs[i] + reloc.offset;

                    match reloc.rtype {
                        R_X86_64_64 => {
                            // Absolute 64-bit
                            let val = (sym_vaddr as i64 + reloc.addend) as u64;
                            let off = reloc.offset as usize;
                            if off + 8 <= self.sections[i].data.len() {
                                self.sections[i].data[off..off + 8]
                                    .copy_from_slice(&val.to_le_bytes());
                            }
                        }
                        R_X86_64_PC32 | R_X86_64_PLT32 => {
                            // PC-relative 32-bit
                            let val = (sym_vaddr as i64 - reloc_vaddr as i64 + reloc.addend) as i32;
                            let off = reloc.offset as usize;
                            if off + 4 <= self.sections[i].data.len() {
                                self.sections[i].data[off..off + 4]
                                    .copy_from_slice(&val.to_le_bytes());
                            }
                        }
                        R_X86_64_32 | R_X86_64_32S => {
                            let val = (sym_vaddr as i64 + reloc.addend) as i32;
                            let off = reloc.offset as usize;
                            if off + 4 <= self.sections[i].data.len() {
                                self.sections[i].data[off..off + 4]
                                    .copy_from_slice(&val.to_le_bytes());
                            }
                        }
                        _ => {} // Unsupported relocation type
                    }
                }
            }
            self.sections[i].relocs = relocs;
        }

        // Find entry point (symbol "main" or "_start")
        let entry_vaddr = self.symbols.get("_start")
            .or_else(|| self.symbols.get("main"))
            .map(|sym| section_vaddrs[sym.section_idx] + sym.offset)
            .unwrap_or(text_seg_vaddr);

        // Build output
        let mut out = vec![0u8; total_file_size as usize];

        // Write ELF header
        let machine = match self.target {
            Target::X86_64 => EM_X86_64,
            Target::I386 => EM_386,
        };
        let mut ehdr = Elf64Header::new_executable(machine, entry_vaddr);
        ehdr.e_phoff = ehdr_size;
        ehdr.e_phnum = num_phdrs as u16;
        ehdr.e_shoff = 0; // No section headers in executable
        ehdr.e_shnum = 0;
        ehdr.e_shstrndx = 0;
        let ehdr_bytes = ehdr.to_bytes();
        out[..ehdr_bytes.len()].copy_from_slice(&ehdr_bytes);

        // Write program headers
        let phdrs_start = ehdr_size as usize;

        // Text segment (RX)
        let text_phdr = Elf64Phdr {
            p_type: PT_LOAD,
            p_flags: PF_R | PF_X,
            p_offset: text_seg_offset,
            p_vaddr: text_seg_vaddr,
            p_paddr: text_seg_vaddr,
            p_filesz: text_seg_size,
            p_memsz: text_seg_size,
            p_align: page_size,
        };
        let bytes = text_phdr.to_bytes();
        out[phdrs_start..phdrs_start + bytes.len()].copy_from_slice(&bytes);

        // Rodata segment (R)
        let rodata_phdr = Elf64Phdr {
            p_type: PT_LOAD,
            p_flags: PF_R,
            p_offset: rodata_seg_offset,
            p_vaddr: rodata_seg_vaddr,
            p_paddr: rodata_seg_vaddr,
            p_filesz: rodata_seg_size,
            p_memsz: rodata_seg_size,
            p_align: page_size,
        };
        let bytes = rodata_phdr.to_bytes();
        let off2 = phdrs_start + 56;
        out[off2..off2 + bytes.len()].copy_from_slice(&bytes);

        // Data segment (RW)
        let data_phdr = Elf64Phdr {
            p_type: PT_LOAD,
            p_flags: PF_R | PF_W,
            p_offset: data_seg_offset,
            p_vaddr: data_seg_vaddr,
            p_paddr: data_seg_vaddr,
            p_filesz: data_seg_size,
            p_memsz: data_seg_size,
            p_align: page_size,
        };
        let bytes = data_phdr.to_bytes();
        let off3 = phdrs_start + 112;
        out[off3..off3 + bytes.len()].copy_from_slice(&bytes);

        // Write section data
        for (i, sec) in self.sections.iter().enumerate() {
            let file_off = section_offsets[i] as usize;
            let end = file_off + sec.data.len();
            if end <= out.len() {
                out[file_off..end].copy_from_slice(&sec.data);
            }
        }

        out
    }
}

// ── ELF parsing helpers ───────────────────────────────────────────────

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
    ])
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
        data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
    ])
}

fn read_i64(data: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes([
        data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
        data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
    ])
}

fn read_cstr<'a>(data: &'a [u8], offset: usize) -> &'a str {
    if offset >= data.len() {
        return "";
    }
    let end = data[offset..].iter().position(|&b| b == 0).unwrap_or(data.len() - offset);
    std::str::from_utf8(&data[offset..offset + end]).unwrap_or("")
}

fn parse_shdr(data: &[u8], offset: usize) -> Elf64Shdr {
    Elf64Shdr {
        sh_name: read_u32(data, offset),
        sh_type: read_u32(data, offset + 4),
        sh_flags: read_u64(data, offset + 8),
        sh_addr: read_u64(data, offset + 16),
        sh_offset: read_u64(data, offset + 24),
        sh_size: read_u64(data, offset + 32),
        sh_link: read_u32(data, offset + 40),
        sh_info: read_u32(data, offset + 44),
        sh_addralign: read_u64(data, offset + 48),
        sh_entsize: read_u64(data, offset + 56),
    }
}

fn align_up(val: u64, align: u64) -> u64 {
    if align == 0 {
        return val;
    }
    (val + align - 1) & !(align - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_align_up() {
        assert_eq!(align_up(0, 4096), 0);
        assert_eq!(align_up(1, 4096), 4096);
        assert_eq!(align_up(4096, 4096), 4096);
        assert_eq!(align_up(4097, 4096), 8192);
    }

    #[test]
    fn test_read_helpers() {
        let data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        assert_eq!(read_u16(&data, 0), 0x0201);
        assert_eq!(read_u32(&data, 0), 0x04030201);
        assert_eq!(read_u64(&data, 0), 0x0807060504030201);
    }

    #[test]
    fn test_read_cstr() {
        let data = b"hello\0world\0";
        assert_eq!(read_cstr(data, 0), "hello");
        assert_eq!(read_cstr(data, 6), "world");
    }

    #[test]
    fn test_link_empty() {
        // Link with no objects should produce a minimal ELF
        let result = link(&[], Target::X86_64);
        assert!(result.len() >= 64); // At least ELF header
        assert_eq!(&result[0..4], &[0x7F, b'E', b'L', b'F']);
    }
}
