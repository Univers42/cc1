// backend/native/elf/writer.rs — ELF object file writer (builtin assembler output).
//
// Produces a relocatable ELF object file (.o) from assembled machine code,
// data sections, symbol table, and relocations.

use super::types::*;
use std::collections::HashMap;

/// A section being built.
#[derive(Debug, Clone)]
pub struct Section {
    pub name: String,
    pub sh_type: u32,
    pub sh_flags: u64,
    pub data: Vec<u8>,
    pub align: u64,
    pub entsize: u64,
    /// Index in the section header table (assigned during finalization).
    pub index: u16,
}

/// A symbol to be placed in the symbol table.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub section_idx: u16,
    pub value: u64,
    pub size: u64,
    pub binding: u8,
    pub sym_type: u8,
}

/// A relocation entry.
#[derive(Debug, Clone)]
pub struct Relocation {
    /// Section that contains the relocation target.
    pub section_name: String,
    /// Offset within the section.
    pub offset: u64,
    /// Symbol name being referenced.
    pub symbol: String,
    /// Relocation type.
    pub rel_type: u32,
    /// Addend.
    pub addend: i64,
}

/// ELF object file writer.
pub struct ElfWriter {
    machine: u16,
    sections: Vec<Section>,
    symbols: Vec<Symbol>,
    relocations: Vec<Relocation>,
    strtab: Vec<u8>,
    shstrtab: Vec<u8>,
    string_map: HashMap<String, u32>,
    shstring_map: HashMap<String, u32>,
}

impl ElfWriter {
    pub fn new(machine: u16) -> Self {
        let mut w = Self {
            machine,
            sections: Vec::new(),
            symbols: Vec::new(),
            relocations: Vec::new(),
            strtab: vec![0], // first byte is null
            shstrtab: vec![0],
            string_map: HashMap::new(),
            shstring_map: HashMap::new(),
        };
        // Null section (index 0)
        w.sections.push(Section {
            name: String::new(),
            sh_type: SHT_NULL,
            sh_flags: 0,
            data: Vec::new(),
            align: 0,
            entsize: 0,
            index: 0,
        });
        w
    }

    /// Add or get a section, returning its index.
    pub fn add_section(&mut self, name: &str, sh_type: u32, flags: u64, align: u64) -> usize {
        // Check if section already exists
        for (i, s) in self.sections.iter().enumerate() {
            if s.name == name {
                return i;
            }
        }
        let idx = self.sections.len();
        self.sections.push(Section {
            name: name.to_string(),
            sh_type,
            sh_flags: flags,
            data: Vec::new(),
            align,
            entsize: 0,
            index: idx as u16,
        });
        idx
    }

    /// Get mutable reference to a section's data.
    pub fn section_data(&mut self, idx: usize) -> &mut Vec<u8> {
        &mut self.sections[idx].data
    }

    /// Current size of a section.
    pub fn section_size(&self, idx: usize) -> usize {
        self.sections[idx].data.len()
    }

    /// Add a symbol.
    pub fn add_symbol(&mut self, sym: Symbol) {
        self.symbols.push(sym);
    }

    /// Add a relocation.
    pub fn add_relocation(&mut self, reloc: Relocation) {
        self.relocations.push(reloc);
    }

    /// Intern a string in the string table, return its offset.
    pub fn intern_string(&mut self, s: &str) -> u32 {
        if let Some(&off) = self.string_map.get(s) {
            return off;
        }
        let off = self.strtab.len() as u32;
        self.strtab.extend_from_slice(s.as_bytes());
        self.strtab.push(0);
        self.string_map.insert(s.to_string(), off);
        off
    }

    /// Intern a string in the section header string table.
    fn intern_shstring(&mut self, s: &str) -> u32 {
        if let Some(&off) = self.shstring_map.get(s) {
            return off;
        }
        let off = self.shstrtab.len() as u32;
        self.shstrtab.extend_from_slice(s.as_bytes());
        self.shstrtab.push(0);
        self.shstring_map.insert(s.to_string(), off);
        off
    }

    /// Finalize and emit the ELF object file as bytes.
    pub fn finalize(&mut self) -> Vec<u8> {
        let mut output = Vec::with_capacity(4096);

        // === Build strtab and shstrtab sections ===

        // Intern section names into shstrtab
        let section_names: Vec<String> = self.sections.iter().map(|s| s.name.clone()).collect();
        let mut shname_offsets = Vec::new();
        for name in &section_names {
            shname_offsets.push(self.intern_shstring(name));
        }

        // Intern symbol names into strtab
        let sym_names: Vec<String> = self.symbols.iter().map(|s| s.name.clone()).collect();
        let sym_name_offsets: Vec<u32> = sym_names.iter()
            .map(|name| self.intern_string(name))
            .collect();

        // Add strtab section
        let strtab_idx = self.add_section(".strtab", SHT_STRTAB, 0, 1);
        shname_offsets.push(self.intern_shstring(".strtab"));
        self.sections[strtab_idx].data = self.strtab.clone();

        // Add shstrtab section
        let shstrtab_name_off = self.intern_shstring(".shstrtab");
        let shstrtab_idx = self.add_section(".shstrtab", SHT_STRTAB, 0, 1);
        shname_offsets.push(shstrtab_name_off);
        self.sections[shstrtab_idx].data = self.shstrtab.clone();

        // Build symtab
        let mut symtab_data = Vec::new();
        // Null symbol
        symtab_data.extend_from_slice(&Elf64Sym::new(0, 0, SHN_UNDEF, 0, 0).to_bytes());
        // Local symbols first, then global
        let mut locals = Vec::new();
        let mut globals = Vec::new();
        for (i, sym) in self.symbols.iter().enumerate() {
            let entry = Elf64Sym::new(
                sym_name_offsets[i],
                elf64_st_info(sym.binding, sym.sym_type),
                sym.section_idx,
                sym.value,
                sym.size,
            );
            if sym.binding == STB_LOCAL {
                locals.push(entry);
            } else {
                globals.push(entry);
            }
        }
        let first_global = (locals.len() + 1) as u32; // +1 for null symbol
        for sym in &locals {
            symtab_data.extend_from_slice(&sym.to_bytes());
        }
        for sym in &globals {
            symtab_data.extend_from_slice(&sym.to_bytes());
        }

        let symtab_idx = self.add_section(".symtab", SHT_SYMTAB, 0, 8);
        self.sections[symtab_idx].entsize = 24;
        self.sections[symtab_idx].data = symtab_data;
        shname_offsets.push(self.intern_shstring(".symtab"));

        // === Build relocation sections ===
        // Group relocations by target section
        let mut rela_groups: HashMap<String, Vec<&Relocation>> = HashMap::new();
        for reloc in &self.relocations {
            rela_groups.entry(reloc.section_name.clone())
                .or_default()
                .push(reloc);
        }
        // (TODO: actually build .rela sections — simplified for now)

        // === Layout pass: assign offsets ===
        let ehdr_size = 64u64;
        let mut offset = ehdr_size;

        // Section data offsets
        let mut section_offsets = Vec::new();
        for section in &self.sections {
            if section.sh_type == SHT_NULL {
                section_offsets.push(0u64);
                continue;
            }
            let align = section.align.max(1);
            offset = (offset + align - 1) & !(align - 1);
            section_offsets.push(offset);
            offset += section.data.len() as u64;
        }

        // Section header table offset
        offset = (offset + 7) & !7; // align to 8
        let shoff = offset;
        let shnum = self.sections.len() as u16;

        // === Write ELF header ===
        let mut header = Elf64Header::new_relocatable(self.machine);
        header.e_shoff = shoff;
        header.e_shnum = shnum;
        header.e_shstrndx = shstrtab_idx as u16;

        // Set symtab link to strtab
        // (We'd need to update the section header after building, but for
        // now we bake it into the section.)

        output.extend_from_slice(&header.to_bytes());

        // === Write section data ===
        for (i, section) in self.sections.iter().enumerate() {
            if section.sh_type == SHT_NULL {
                continue;
            }
            // Pad to alignment
            let target = section_offsets[i] as usize;
            while output.len() < target {
                output.push(0);
            }
            output.extend_from_slice(&section.data);
        }

        // Pad to section header table
        while output.len() < shoff as usize {
            output.push(0);
        }

        // === Write section header table ===
        for (i, section) in self.sections.iter().enumerate() {
            let sh_name = if i < shname_offsets.len() {
                shname_offsets[i]
            } else {
                0
            };
            let mut shdr = Elf64Shdr::null();
            shdr.sh_name = sh_name;
            shdr.sh_type = section.sh_type;
            shdr.sh_flags = section.sh_flags;
            shdr.sh_offset = section_offsets[i];
            shdr.sh_size = section.data.len() as u64;
            shdr.sh_addralign = section.align;
            shdr.sh_entsize = section.entsize;

            // Link symtab to strtab
            if section.sh_type == SHT_SYMTAB {
                shdr.sh_link = strtab_idx as u32;
                shdr.sh_info = first_global;
            }

            output.extend_from_slice(&shdr.to_bytes());
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_elf_writer_minimal() {
        let mut w = ElfWriter::new(EM_X86_64);

        // Add .text section
        let text_idx = w.add_section(".text", SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR, 16);
        w.section_data(text_idx).extend_from_slice(&[
            0x55,                   // push %rbp
            0x48, 0x89, 0xe5,      // mov %rsp, %rbp
            0xb8, 0x00, 0x00, 0x00, 0x00, // mov $0, %eax
            0x5d,                   // pop %rbp
            0xc3,                   // ret
        ]);

        // Add a symbol
        w.add_symbol(Symbol {
            name: "main".into(),
            section_idx: text_idx as u16,
            value: 0,
            size: w.section_size(text_idx) as u64,
            binding: STB_GLOBAL,
            sym_type: STT_FUNC,
        });

        let bytes = w.finalize();

        // Check ELF magic
        assert_eq!(&bytes[0..4], &ELF_MAGIC);
        // Check it's reasonably sized (header + sections + section headers)
        assert!(bytes.len() > 64);
    }

    #[test]
    fn test_elf_writer_with_data() {
        let mut w = ElfWriter::new(EM_X86_64);

        let text_idx = w.add_section(".text", SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR, 16);
        w.section_data(text_idx).extend_from_slice(&[0xc3]);

        let rodata_idx = w.add_section(".rodata", SHT_PROGBITS, SHF_ALLOC | SHF_MERGE, 1);
        w.section_data(rodata_idx).extend_from_slice(b"Hello, world!\0");

        w.add_symbol(Symbol {
            name: "_start".into(),
            section_idx: text_idx as u16,
            value: 0,
            size: 1,
            binding: STB_GLOBAL,
            sym_type: STT_FUNC,
        });

        let bytes = w.finalize();
        assert_eq!(&bytes[0..4], &ELF_MAGIC);
    }
}
