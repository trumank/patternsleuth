use std::{collections::HashMap, mem, ops::Range};

use crate::{Memory, MemoryAccessError, MemoryTrait, NamedMemorySection, RuntimeFunction};

use super::{Image, ImageType};
use gimli::{BaseAddresses, CieOrFde, EhFrame, EhFrameHdr, NativeEndian, UnwindSection};

#[cfg(feature = "symbols")]
use crate::uesym;
use anyhow::{Context, Error, Result, bail};
use object::{
    Endianness, File, Object, ObjectSection, SectionKind, elf::ProgramHeader64,
    read::elf::ElfFile64, read::elf::ProgramHeader,
};

pub struct ElfImage {
    pub functions: Option<Vec<Range<u64>>>,
}

#[allow(dead_code)]
struct Elf64Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

impl ElfImage {
    pub fn get_function(
        &self,
        image: &Image<'_>,
        address: u64,
    ) -> Result<Option<RuntimeFunction>, MemoryAccessError> {
        self.get_root_function(image, address)
    }
    pub fn get_root_function(
        &self,
        _image: &Image<'_>,
        address: u64,
    ) -> Result<Option<RuntimeFunction>, MemoryAccessError> {
        self.get_root_function_range(_image, address).map(|range| {
            range.map(|r| RuntimeFunction {
                range: r,
                unwind: 0,
            })
        })
    }
    pub fn get_root_function_range(
        &self,
        _image: &Image<'_>,
        address: u64,
    ) -> Result<Option<Range<u64>>, MemoryAccessError> {
        let x = self.functions.as_ref().unwrap();
        Ok(x.iter().find(|p| p.contains(&address)).cloned())
    }
    pub fn get_child_functions(
        &self,
        image: &Image<'_>,
        address: u64,
    ) -> Result<Vec<RuntimeFunction>, MemoryAccessError> {
        match self.get_function(image, address) {
            Ok(Some(f)) => Ok(vec![f]),
            Ok(None) => Ok(vec![]),
            Err(e) => Err(e),
        }
    }
    pub fn get_root_functions(&self, _: &Image<'_>) -> Result<Vec<Range<u64>>, MemoryAccessError> {
        Ok(self.functions.as_ref().unwrap().to_vec())
    }
}

// read_inner
impl ElfImage {
    /// Read and parse ELF object, using data from memory
    pub fn read_inner_memory<'data, P: AsRef<std::path::Path>>(
        base_address: u64,
        #[allow(unused_variables)] exe_path: Option<P>,
        linked: bool,
        memory: Memory<'data>,
        object: ElfFile64<'data>,
    ) -> Result<Image<'data>, anyhow::Error> {
        // start to parse eh_frame

        let endian = object.endian();
        let phdr_map = |segment: &ProgramHeader64<Endianness>| Elf64Phdr {
            p_type: segment.p_type(endian),
            p_flags: segment.p_flags(endian),
            p_offset: segment.p_offset(endian),
            p_vaddr: segment.p_vaddr(endian),
            p_paddr: segment.p_paddr(endian),
            p_filesz: segment.p_filesz(endian),
            p_memsz: segment.p_memsz(endian),
            p_align: segment.p_align(endian),
        };

        let get_offset = |segment: &Elf64Phdr| {
            if linked {
                // for Elf loaded in memory, the map starts from smallest p_vaddr
                (segment.p_vaddr + base_address)..(segment.p_vaddr + segment.p_memsz + base_address)
            } else {
                // for Elf file loaded as file, the map starts from 0
                segment.p_offset..(segment.p_offset + segment.p_filesz)
            }
        };

        let functions = if linked {
            // try get address from phdr only when it's loaded in memory
            // otherwise, use section to avoid possible relocation problem with
            // eh_frame_hdr.
            // I assume if the encoding for the pointer is DW_EH_PE_indirect
            // that address might need to be filled by relocation, so if
            // the elf is opened as file, the address is not ready to use.
            let eh_frame_hdr = object
                .raw_segments()
                .iter()
                .find(|segment| segment.p_type(endian) == object::elf::PT_GNU_EH_FRAME)
                .map(phdr_map)
                .context("Cannot find PT_GNU_EH_FRAME phdr");

            eh_frame_hdr
                .map(|p| -> Result<Vec<Range<u64>>, Error> {
                    //eprintln!("Found GNU_EH_FRAME");
                    let text_vaddr = memory
                        .sections()
                        .iter()
                        .find(|s| s.name == ".text")
                        .context("Cannot find .text section")?
                        .address();
                    let ehframe_hdr_start = base_address + p.p_vaddr;
                    let bases = BaseAddresses::default()
                        .set_eh_frame_hdr(ehframe_hdr_start as _)
                        .set_text((text_vaddr + base_address) as _);
                    let ehframe_hdr_range = get_offset(&p);

                    let ehframe_hdr: gimli::ParsedEhFrameHdr<
                        gimli::EndianSlice<'_, gimli::LittleEndian>,
                    > = EhFrameHdr::new(memory.range(ehframe_hdr_range)?, NativeEndian)
                        .parse(&bases, mem::size_of::<u64>() as _)
                        .context("Failed to parse eh_frame_hdr")?;

                    let ehframe_realaddr = match ehframe_hdr.eh_frame_ptr() {
                        gimli::Pointer::Direct(ptr) => ptr,
                        // should I subtract base_address?
                        gimli::Pointer::Indirect(ptr) => memory.u64_le(ptr as _)? as _,
                    };

                    let bases = bases.set_eh_frame(ehframe_realaddr as _);
                    let eh_frame =
                        EhFrame::new(memory.range_from(ehframe_realaddr..)?, NativeEndian);

                    let mut entries = eh_frame.entries(&bases);

                    let mut result = Vec::<Range<u64>>::new();
                    while let Some(entry) = entries.next().context("Iter over entry failed")? {
                        match entry {
                            CieOrFde::Fde(partial) => {
                                let fde = partial
                                    .parse(&mut EhFrame::cie_from_offset)
                                    .context("Failed parse fde item")?;
                                // right now it's real address
                                let start = fde.initial_address();
                                let len = fde.len();
                                result.push(start..(start + len));
                            }
                            CieOrFde::Cie(_) => {}
                        }
                    }
                    result.sort_by(|a, b| a.start.cmp(&b.start));
                    Ok(result)
                })
                .context("Cannot find eh_frame")?
        } else {
            let eh_frame = object
                .section_by_name(".eh_frame")
                .context("Cannot find section .eh_frame in elf")?;
            let eh_frame_hdr = object
                .section_by_name(".eh_frame_hdr")
                .context("Cannot find section .eh_frame_hdr in elf")?;
            let text = object.section_by_name(".text").unwrap();
            let bases = gimli::BaseAddresses::default()
                .set_eh_frame_hdr(eh_frame_hdr.address() as _)
                .set_eh_frame(eh_frame.address())
                .set_text(text.address() as _);
            let eh_frameparsed = EhFrame::new(eh_frame.data().unwrap(), NativeEndian);
            let mut entries = eh_frameparsed.entries(&bases);

            let mut result = Vec::<Range<u64>>::new();

            while let Some(entry) = entries.next().context("Iter over entry failed")? {
                match entry {
                    CieOrFde::Fde(partial) => {
                        let fde = partial
                            .parse(&mut EhFrame::cie_from_offset)
                            .context("Failed parse fde item")?;
                        // right now it's real address
                        let start = fde.initial_address();
                        let len = fde.len();
                        result.push(start..(start + len));
                    }
                    CieOrFde::Cie(_) => {}
                }
            }
            result.sort_by(|a, b| a.start.cmp(&b.start));
            // eprintln!("Found {} fde", result.len());
            Ok(result)
        }?;

        #[cfg(feature = "symbols")]
        let symbols = if let Some(exe_path) = exe_path {
            let sym_path = exe_path.as_ref().with_extension("sym");
            sym_path
                .exists()
                .then(|| -> Result<HashMap<_, _>> {
                    let syms = uesym::dump_ue_symbols(sym_path, base_address)?;
                    Ok((functions
                        .iter()
                        .flat_map(|f| -> Option<(u64, crate::symbols::Symbol)> {
                            Some((f.start, syms.get(&f.start)?.clone()))
                        }))
                    .collect())
                })
                .transpose()?
        } else {
            None
        };
        Ok(Image {
            base_address,
            memory,
            #[cfg(feature = "symbols")]
            symbols,
            imports: HashMap::default(),
            image_type: ImageType::ElfImage(ElfImage {
                functions: Some(functions),
            }),
        })
    }

    /// Read and parse ELF object, using data from object.data()
    pub fn read_inner<P: AsRef<std::path::Path>>(
        base_addr: Option<u64>,
        exe_path: Option<P>,
        _cache_functions: bool,
        object: object::File<'_>,
    ) -> Result<Image<'_>, anyhow::Error> {
        let base_address = base_addr.unwrap_or(object.relative_address_base());
        let linked = base_addr.is_some();
        let calc_kind = |flag: u32| {
            if flag & object::elf::PF_X == object::elf::PF_X {
                SectionKind::Text
            } else if flag & object::elf::PF_W == object::elf::PF_W {
                SectionKind::Data
            } else if flag & object::elf::PF_R == object::elf::PF_R {
                SectionKind::ReadOnlyData
            } else {
                SectionKind::Unknown
            }
        };

        // the elf may not contains section table if it's in memory, use phdr instead.
        if let File::Elf64(object) = object {
            let endian = object.endian();
            let phdr_map = |segment: &ProgramHeader64<Endianness>| Elf64Phdr {
                p_type: segment.p_type(endian),
                p_flags: segment.p_flags(endian),
                p_offset: segment.p_offset(endian),
                p_vaddr: segment.p_vaddr(endian),
                p_paddr: segment.p_paddr(endian),
                p_filesz: segment.p_filesz(endian),
                p_memsz: segment.p_memsz(endian),
                p_align: segment.p_align(endian),
            };
            let phdrs = object
                .raw_segments()
                .iter()
                .filter(|segment| segment.p_type(endian) == object::elf::PT_LOAD)
                .map(phdr_map)
                .collect::<Vec<_>>();

            let _map_end = phdrs
                .iter()
                .map(|p| p.p_vaddr + p.p_memsz)
                .max()
                .unwrap_or_default();
            let map_start = phdrs.iter().map(|p| p.p_vaddr).min().unwrap_or_default();

            let get_offset = |segment: &Elf64Phdr| {
                if linked {
                    // for Elf loaded in memory, the map starts from smallest p_vaddr
                    (segment.p_vaddr - map_start) as usize
                        ..(segment.p_vaddr + segment.p_memsz - map_start) as usize
                } else {
                    // for Elf file loaded as file, the map starts from 0
                    segment.p_offset as usize..(segment.p_offset + segment.p_filesz) as usize
                }
            };

            let entrypoint = object.entry();
            let sections = phdrs
                .iter()
                .enumerate()
                .map(|(idx, segment)| {
                    let vaddr_range = segment.p_vaddr..(segment.p_vaddr + segment.p_filesz);
                    let offset_range = get_offset(segment);
                    let section_name = if !vaddr_range.contains(&entrypoint) {
                        format!("FakeSection {}", idx + 1)
                    } else {
                        ".text".to_owned()
                    };
                    NamedMemorySection::new(
                        section_name,
                        base_address + segment.p_vaddr,
                        calc_kind(segment.p_flags),
                        &object.data()[offset_range],
                    )
                })
                .collect::<Vec<_>>();

            let memory = Memory { sections };

            Self::read_inner_memory(base_address, exe_path, linked, memory, object)
        } else {
            bail!("Not a elf file")
        }
    }
}

/*



        let eh_frame = object.section_by_name(".eh_frame").unwrap();
        let eh_frame_hdr = object.section_by_name(".eh_frame_hdr").unwrap();
        let text = object.section_by_name(".text").unwrap();
        let bases = gimli::BaseAddresses::default()
                .set_eh_frame_hdr(eh_frame_hdr.address() as _)
                .set_eh_frame(eh_frame.address())
                .set_text(text.address() as _);
        let eh_frameparsed = EhFrame::new(
            eh_frame.data().unwrap(),
            NativeEndian
        );
        let mut entries = eh_frameparsed.entries(&bases);

        let mut result = Vec::<Range<usize>>::new();
        let mut syms = HashMap::default();

        while let Some(entry) = entries.next().context("Iter over entry failed")? {
            match entry {
                CieOrFde::Fde(partial) => {
                    let fde = partial
                        .parse(&mut EhFrame::cie_from_offset)
                        .context("Failed parse fde item")?;
                    // right now it's real address
                    let start = fde.initial_address() as usize;
                    let len = fde.len() as usize;
                    result.push(start .. (start + len));
                    syms.insert(start, format!("sub_{}", start));
                }
                CieOrFde::Cie(_) => {},
            }
        }
        result.sort_by(|a,b| a.start.cmp(&b.start));

*/
