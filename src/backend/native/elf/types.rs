// backend/native/elf/types.rs — ELF binary format constants and structures.
//
// References: ELF specification (Tool Interface Standard, Portable Formats).

#![allow(dead_code)]

// ── ELF Constants ─────────────────────────────────────────────────────

// ELF magic
pub const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

// ELF classes
pub const ELFCLASS32: u8 = 1;
pub const ELFCLASS64: u8 = 2;

// ELF data encoding
pub const ELFDATA2LSB: u8 = 1; // Little-endian
pub const ELFDATA2MSB: u8 = 2; // Big-endian

// ELF version
pub const EV_CURRENT: u8 = 1;

// ELF OS/ABI
pub const ELFOSABI_NONE: u8 = 0;
pub const ELFOSABI_LINUX: u8 = 3;

// ELF types
pub const ET_NONE: u16 = 0;
pub const ET_REL: u16 = 1;    // Relocatable file (.o)
pub const ET_EXEC: u16 = 2;   // Executable file
pub const ET_DYN: u16 = 3;    // Shared object
pub const ET_CORE: u16 = 4;   // Core file

// ELF machine types
pub const EM_386: u16 = 3;
pub const EM_X86_64: u16 = 62;
pub const EM_AARCH64: u16 = 183;
pub const EM_RISCV: u16 = 243;

// Section types
pub const SHT_NULL: u32 = 0;
pub const SHT_PROGBITS: u32 = 1;
pub const SHT_SYMTAB: u32 = 2;
pub const SHT_STRTAB: u32 = 3;
pub const SHT_RELA: u32 = 4;
pub const SHT_HASH: u32 = 5;
pub const SHT_DYNAMIC: u32 = 6;
pub const SHT_NOTE: u32 = 7;
pub const SHT_NOBITS: u32 = 8;
pub const SHT_REL: u32 = 9;

// Section flags
pub const SHF_WRITE: u64 = 0x1;
pub const SHF_ALLOC: u64 = 0x2;
pub const SHF_EXECINSTR: u64 = 0x4;
pub const SHF_MERGE: u64 = 0x10;
pub const SHF_STRINGS: u64 = 0x20;
pub const SHF_INFO_LINK: u64 = 0x40;
pub const SHF_GROUP: u64 = 0x200;
pub const SHF_TLS: u64 = 0x400;

// Symbol binding
pub const STB_LOCAL: u8 = 0;
pub const STB_GLOBAL: u8 = 1;
pub const STB_WEAK: u8 = 2;

// Symbol type
pub const STT_NOTYPE: u8 = 0;
pub const STT_OBJECT: u8 = 1;
pub const STT_FUNC: u8 = 2;
pub const STT_SECTION: u8 = 3;
pub const STT_FILE: u8 = 4;

// Symbol visibility
pub const STV_DEFAULT: u8 = 0;
pub const STV_INTERNAL: u8 = 1;
pub const STV_HIDDEN: u8 = 2;
pub const STV_PROTECTED: u8 = 3;

// Special section indices
pub const SHN_UNDEF: u16 = 0;
pub const SHN_ABS: u16 = 0xFFF1;
pub const SHN_COMMON: u16 = 0xFFF2;

// Program header types
pub const PT_NULL: u32 = 0;
pub const PT_LOAD: u32 = 1;
pub const PT_DYNAMIC: u32 = 2;
pub const PT_INTERP: u32 = 3;
pub const PT_NOTE: u32 = 4;
pub const PT_PHDR: u32 = 6;

// Program header flags
pub const PF_X: u32 = 0x1;
pub const PF_W: u32 = 0x2;
pub const PF_R: u32 = 0x4;

// x86-64 relocation types
pub const R_X86_64_NONE: u32 = 0;
pub const R_X86_64_64: u32 = 1;
pub const R_X86_64_PC32: u32 = 2;
pub const R_X86_64_GOT32: u32 = 3;
pub const R_X86_64_PLT32: u32 = 4;
pub const R_X86_64_32: u32 = 10;
pub const R_X86_64_32S: u32 = 11;

// ── ELF64 Structures ─────────────────────────────────────────────────

/// ELF64 file header.
#[derive(Debug, Clone)]
pub struct Elf64Header {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

impl Elf64Header {
    pub fn new_relocatable(machine: u16) -> Self {
        let mut ident = [0u8; 16];
        ident[0..4].copy_from_slice(&ELF_MAGIC);
        ident[4] = ELFCLASS64;
        ident[5] = ELFDATA2LSB;
        ident[6] = EV_CURRENT;
        ident[7] = ELFOSABI_NONE;

        Self {
            e_ident: ident,
            e_type: ET_REL,
            e_machine: machine,
            e_version: 1,
            e_entry: 0,
            e_phoff: 0,
            e_shoff: 0, // filled later
            e_flags: 0,
            e_ehsize: 64,
            e_phentsize: 0,
            e_phnum: 0,
            e_shentsize: 64,
            e_shnum: 0, // filled later
            e_shstrndx: 0, // filled later
        }
    }

    pub fn new_executable(machine: u16, entry: u64) -> Self {
        let mut h = Self::new_relocatable(machine);
        h.e_type = ET_EXEC;
        h.e_entry = entry;
        h.e_phentsize = 56;
        h
    }

    /// Serialize to bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(&self.e_ident);
        buf.extend_from_slice(&self.e_type.to_le_bytes());
        buf.extend_from_slice(&self.e_machine.to_le_bytes());
        buf.extend_from_slice(&self.e_version.to_le_bytes());
        buf.extend_from_slice(&self.e_entry.to_le_bytes());
        buf.extend_from_slice(&self.e_phoff.to_le_bytes());
        buf.extend_from_slice(&self.e_shoff.to_le_bytes());
        buf.extend_from_slice(&self.e_flags.to_le_bytes());
        buf.extend_from_slice(&self.e_ehsize.to_le_bytes());
        buf.extend_from_slice(&self.e_phentsize.to_le_bytes());
        buf.extend_from_slice(&self.e_phnum.to_le_bytes());
        buf.extend_from_slice(&self.e_shentsize.to_le_bytes());
        buf.extend_from_slice(&self.e_shnum.to_le_bytes());
        buf.extend_from_slice(&self.e_shstrndx.to_le_bytes());
        buf
    }
}

/// ELF64 section header.
#[derive(Debug, Clone)]
pub struct Elf64Shdr {
    pub sh_name: u32,
    pub sh_type: u32,
    pub sh_flags: u64,
    pub sh_addr: u64,
    pub sh_offset: u64,
    pub sh_size: u64,
    pub sh_link: u32,
    pub sh_info: u32,
    pub sh_addralign: u64,
    pub sh_entsize: u64,
}

impl Elf64Shdr {
    pub fn null() -> Self {
        Self {
            sh_name: 0,
            sh_type: SHT_NULL,
            sh_flags: 0,
            sh_addr: 0,
            sh_offset: 0,
            sh_size: 0,
            sh_link: 0,
            sh_info: 0,
            sh_addralign: 0,
            sh_entsize: 0,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(&self.sh_name.to_le_bytes());
        buf.extend_from_slice(&self.sh_type.to_le_bytes());
        buf.extend_from_slice(&self.sh_flags.to_le_bytes());
        buf.extend_from_slice(&self.sh_addr.to_le_bytes());
        buf.extend_from_slice(&self.sh_offset.to_le_bytes());
        buf.extend_from_slice(&self.sh_size.to_le_bytes());
        buf.extend_from_slice(&self.sh_link.to_le_bytes());
        buf.extend_from_slice(&self.sh_info.to_le_bytes());
        buf.extend_from_slice(&self.sh_addralign.to_le_bytes());
        buf.extend_from_slice(&self.sh_entsize.to_le_bytes());
        buf
    }
}

/// ELF64 symbol table entry.
#[derive(Debug, Clone)]
pub struct Elf64Sym {
    pub st_name: u32,
    pub st_info: u8,
    pub st_other: u8,
    pub st_shndx: u16,
    pub st_value: u64,
    pub st_size: u64,
}

impl Elf64Sym {
    pub fn new(name: u32, info: u8, shndx: u16, value: u64, size: u64) -> Self {
        Self {
            st_name: name,
            st_info: info,
            st_other: STV_DEFAULT,
            st_shndx: shndx,
            st_value: value,
            st_size: size,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(24);
        buf.extend_from_slice(&self.st_name.to_le_bytes());
        buf.push(self.st_info);
        buf.push(self.st_other);
        buf.extend_from_slice(&self.st_shndx.to_le_bytes());
        buf.extend_from_slice(&self.st_value.to_le_bytes());
        buf.extend_from_slice(&self.st_size.to_le_bytes());
        buf
    }
}

/// ELF64 RELA relocation entry.
#[derive(Debug, Clone)]
pub struct Elf64Rela {
    pub r_offset: u64,
    pub r_info: u64,
    pub r_addend: i64,
}

impl Elf64Rela {
    pub fn new(offset: u64, sym: u32, rel_type: u32, addend: i64) -> Self {
        Self {
            r_offset: offset,
            r_info: ((sym as u64) << 32) | (rel_type as u64),
            r_addend: addend,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(24);
        buf.extend_from_slice(&self.r_offset.to_le_bytes());
        buf.extend_from_slice(&self.r_info.to_le_bytes());
        buf.extend_from_slice(&self.r_addend.to_le_bytes());
        buf
    }
}

/// ELF64 program header.
#[derive(Debug, Clone)]
pub struct Elf64Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

impl Elf64Phdr {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(56);
        buf.extend_from_slice(&self.p_type.to_le_bytes());
        buf.extend_from_slice(&self.p_flags.to_le_bytes());
        buf.extend_from_slice(&self.p_offset.to_le_bytes());
        buf.extend_from_slice(&self.p_vaddr.to_le_bytes());
        buf.extend_from_slice(&self.p_paddr.to_le_bytes());
        buf.extend_from_slice(&self.p_filesz.to_le_bytes());
        buf.extend_from_slice(&self.p_memsz.to_le_bytes());
        buf.extend_from_slice(&self.p_align.to_le_bytes());
        buf
    }
}

/// Helper: encode ST_INFO field.
pub fn elf64_st_info(bind: u8, st_type: u8) -> u8 {
    (bind << 4) | (st_type & 0xf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_elf_header_size() {
        let h = Elf64Header::new_relocatable(EM_X86_64);
        let bytes = h.to_bytes();
        assert_eq!(bytes.len(), 64);
        assert_eq!(&bytes[0..4], &ELF_MAGIC);
    }

    #[test]
    fn test_section_header_size() {
        let sh = Elf64Shdr::null();
        assert_eq!(sh.to_bytes().len(), 64);
    }

    #[test]
    fn test_symbol_entry_size() {
        let sym = Elf64Sym::new(0, 0, 0, 0, 0);
        assert_eq!(sym.to_bytes().len(), 24);
    }

    #[test]
    fn test_rela_entry_size() {
        let rela = Elf64Rela::new(0, 0, R_X86_64_PC32, -4);
        assert_eq!(rela.to_bytes().len(), 24);
    }

    #[test]
    fn test_phdr_size() {
        let phdr = Elf64Phdr {
            p_type: PT_LOAD,
            p_flags: PF_R | PF_X,
            p_offset: 0,
            p_vaddr: 0x400000,
            p_paddr: 0x400000,
            p_filesz: 0x1000,
            p_memsz: 0x1000,
            p_align: 0x1000,
        };
        assert_eq!(phdr.to_bytes().len(), 56);
    }

    #[test]
    fn test_st_info() {
        assert_eq!(elf64_st_info(STB_GLOBAL, STT_FUNC), 0x12);
        assert_eq!(elf64_st_info(STB_LOCAL, STT_OBJECT), 0x01);
    }
}
