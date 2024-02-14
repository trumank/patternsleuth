#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "linux")]
mod linux {
    use core::panic;
    use std::{collections::HashMap, default, mem, ops::Range, ptr::{null, null_mut}};

    use anyhow::{Context, Error, Result};
    use gimli::{BaseAddresses, CieOrFde, EhFrame, EhFrameHdr, NativeEndian, Pointer, UnwindSection};
    use itertools::Itertools;
    use object::{elf::{DT_HASH, DT_STRTAB, DT_SYMTAB}, Object, ObjectSection, SectionKind};

    use crate::{Image, Memory, NamedMemorySection};
    use libc::{c_void, dl_iterate_phdr, dladdr1, readlink, Dl_info, Elf64_Addr, Elf64_Ehdr, Elf64_Phdr, Elf64_Sxword, Elf64_Sym, Elf64_Xword, PF_R, PF_W, PF_X, PT_GNU_EH_FRAME, PT_LOAD, RTLD_DI_LINKMAP};
    
    #[repr(C)]
    #[derive(Debug)]
    pub struct Elf64_Dyn {
        pub d_tag: Elf64_Sxword,
        pub d_val: Elf64_Xword,
    }

    const DT_NUM:usize = 34;

    #[repr(C)]
    #[derive(Debug)]
    pub struct LinkMap {
        pub l_addr: Elf64_Addr,
        pub l_name: *const libc::c_char,
        pub l_ld: *const Elf64_Dyn,
        pub l_next: *const LinkMap,
        pub l_prev: *const LinkMap,
        pub l_real: *const LinkMap,
        pub l_ns: usize,
        pub l_libname: *const libc::c_void,
        pub l_info: [*const Elf64_Dyn; DT_NUM],
    }

    unsafe extern "C" fn dl_iterate_phdr_callback(
        info: *mut libc::dl_phdr_info,
        _size: usize,
        data: *mut std::ffi::c_void,
    ) -> i32{
        let name = unsafe { std::ffi::CStr::from_ptr((*info).dlpi_name) };
        let name = name.to_str().unwrap();
        let image = data as *mut libc::dl_phdr_info;
        //eprintln!("Name: {}", name);
        //eprintln!("BaseAddr: {:08x}", (*info).dlpi_addr);
        if name.is_empty() {
            // find the main
            //eprintln!("Base addr from iter = {:08x}", (*info).dlpi_addr);
            *image = *info;
        }
        0
    }
    
    pub unsafe fn deref_pointer(ptr: Pointer) -> usize {
        match ptr {
            Pointer::Direct(x) => x as _,
            Pointer::Indirect(x) => unsafe { *(x as *const _) },
        }
    }
    
    // in theory, can use the phdr length to limit this size
    pub unsafe fn cast_slice_todo_remove_me<'a>(start: *const u8) -> &'a [u8] {
        let start = start as usize;
        let end = usize::MAX;
        let len = end - start;
        unsafe { std::slice::from_raw_parts(start as *const _, len) }
    }
    

    pub fn read_image<'data>() -> Result<Image<'data>> {
        unsafe {
            let mut info = libc::dl_phdr_info {
                dlpi_addr: 0,
                dlpi_name: null(),
                dlpi_phdr: null(),
                dlpi_phnum: 0,
                dlpi_adds: 0,
                dlpi_subs: 0,
                dlpi_tls_modid: 0,
                dlpi_tls_data: null_mut(),
            };
            dl_iterate_phdr(Some(dl_iterate_phdr_callback), (&mut info) as *mut libc::dl_phdr_info as *mut std::ffi::c_void);
            
            // base addr is the offset to the real map from the vaddr in elf
            let base_addr = (info).dlpi_addr as usize;
            //eprintln!("Base addr {} (should be zero)", base_addr);
            
            //eprintln!("ph num = {}", info.dlpi_phnum);
            let phdr_slice = std::slice::from_raw_parts_mut((info).dlpi_phdr as *mut Elf64_Phdr , (info).dlpi_phnum as usize);
            // print all phdr
            /*for p in phdr_slice.iter() {
                eprintln!("Range: {:08x} -- {:08x} ", p.p_vaddr, p.p_vaddr + p.p_memsz);
                eprintln!("P_TYPE: {} (load = 1)", p.p_type);
            }*/
            let map_end = phdr_slice.iter().filter(|p| p.p_type == PT_LOAD).map(|p|p.p_vaddr+p.p_memsz).max().unwrap_or_default() as usize;
            let map_start = phdr_slice.iter().filter(|p| p.p_type == PT_LOAD).map(|p|p.p_vaddr).min().unwrap_or_default() as usize;
            
            //eprintln!("Map Start -- Map End = {:08x} -- {:08x}", map_start, map_end);
            // find entrypoint
            let ehdr = (base_addr + map_start) as * const Elf64_Ehdr;
            let entrypoint_vaddr = (*ehdr).e_entry;
            //eprintln!("entrypoint_vaddr = {:08x}", entrypoint_vaddr);
            
            let memory = std::slice::from_raw_parts(
                    (base_addr + map_start) as *mut u8,
                    map_end - map_start
            );
            
            let calc_kind = |flag: u32| {
                if flag & PF_X == PF_X {
                    SectionKind::Text
                } else if flag & PF_W == PF_W  {
                    SectionKind::Data
                } else if flag & PF_R == PF_R {
                    SectionKind::ReadOnlyData
                } else {
                    SectionKind::Unknown
                }
            };

            let mut text_vaddr = 0 as usize;
            let sections = phdr_slice.iter().filter(|p| p.p_type == PT_LOAD).enumerate().map(|(idx, p)| {
                let vrange = p.p_vaddr .. (p.p_vaddr + p.p_memsz);
                let addr = p.p_vaddr as usize - map_start;
                let size = p.p_memsz as usize;
                let section_name =  if ! vrange.contains(&entrypoint_vaddr) { format!("FakeSection {}", idx + 1) } else {".text".to_owned()};
                text_vaddr = p.p_vaddr as usize;
                //eprintln!("Section {} {:08x} -- {:08x} Size = {:08x}", section_name, p.p_vaddr, p.p_vaddr + p.p_memsz, p.p_memsz);
                NamedMemorySection::new(
                    section_name,
                    p.p_vaddr as usize + base_addr,
                    calc_kind(p.p_flags),
                    &memory[addr..addr + size]
                )
            }).collect_vec();

            // collect map info from /proc/self/maps
            //let mut map = std::fs::read_to_string("/proc/self/maps").context("Cannot read /proc/self/maps")?;
            //eprintln!("Map: {}", map);

            // print readlink(/proc/self/exe)
           // panic!("Bye! {:?}",path);

            sections.iter().find(|s| s.kind == SectionKind::Text);

            let memory = Memory {
                sections: sections
            };
            //eprintln!("Finding GNU_EH_FRAME");
            // find GNU_EH_FRAME for debug info
            let mut ehframe = phdr_slice.iter().find(|p| p.p_type == PT_GNU_EH_FRAME).map(|p| -> Result<Vec<Range<usize>>, Error>{
                //eprintln!("Found GNU_EH_FRAME");
                let ehframe_hdr_start = base_addr + p.p_vaddr as usize;
                let bases = BaseAddresses::default()
                    .set_eh_frame_hdr(ehframe_hdr_start as _)
                    .set_text((text_vaddr + base_addr) as _);
                let ehframe_hdr = EhFrameHdr::new(
                    std::slice::from_raw_parts(ehframe_hdr_start as *const u8, p.p_memsz as usize), 
                    NativeEndian
                ).parse(&bases, mem::size_of::<usize>() as _)
                 .context("Failed to parse ehframe")?;

                let eframe = deref_pointer(ehframe_hdr.eh_frame_ptr());
                let bases = bases.set_eh_frame(eframe as _);
                let eh_frame = EhFrame::new(
                    cast_slice_todo_remove_me(eframe as usize as _), 
                    NativeEndian
                );

                let mut entries = eh_frame.entries(&bases);
                
                let mut result = Vec::<Range<usize>>::new();
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
                        }
                        CieOrFde::Cie(_) => {},
                    }
                }
                Ok(result)
            }).context("Cannot find eh_frame")??;
            //eprintln!("Total {} Functions found.", ehframe.len());
            ehframe.sort_by(|a,b| {
                a.start.cmp(&b.start)
            });
            Ok(Image {
                base_address: map_start,
                memory,
                functions: Some(ehframe),
                symbols: None,
                exception_children_cache: HashMap::default(),
            })
        }
    }
}

#[cfg(windows)]
pub use windows::*;

#[cfg(windows)]
mod windows {
    use anyhow::{Context, Result};
    use object::{Object, ObjectSection};
    use windows::Win32::System::{
        LibraryLoader::GetModuleHandleA,
        ProcessStatus::{GetModuleInformation, MODULEINFO},
        Threading::GetCurrentProcess,
    };

    use crate::{Image, Memory};

    pub fn read_image<'data>() -> Result<Image<'data>> {
        let main_module =
            unsafe { GetModuleHandleA(None) }.context("could not find main module")?;
        let process = unsafe { GetCurrentProcess() };

        let mut mod_info = MODULEINFO::default();
        unsafe {
            GetModuleInformation(
                process,
                main_module,
                &mut mod_info as *mut _,
                std::mem::size_of::<MODULEINFO>() as u32,
            )?
        };

        let memory = unsafe {
            std::slice::from_raw_parts(
                mod_info.lpBaseOfDll as *mut u8,
                mod_info.SizeOfImage as usize,
            )
        };

        let object = object::File::parse(memory)?;

        let image_base_address = object.relative_address_base() as usize;

        let mut sections = vec![];
        for section in object.sections() {
            let addr = section.address() as usize - image_base_address;
            let size = section.size() as usize;
            sections.push((section, &memory[addr..addr + size]));
        }

        let memory = Memory::new_internal_data(sections)?;

        Image::read_inner::<String>(None, false, memory, object)
    }
}
