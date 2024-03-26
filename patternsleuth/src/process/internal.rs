#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "linux")]
mod linux {
    use std::ptr::{null, null_mut};

    use anyhow::Result;

    use crate::Image;
    use libc::{dl_iterate_phdr, Elf64_Addr, Elf64_Phdr, Elf64_Sxword, Elf64_Xword, PT_LOAD};

    #[repr(C)]
    #[derive(Debug)]
    pub struct Elf64Dyn {
        pub d_tag: Elf64_Sxword,
        pub d_val: Elf64_Xword,
    }

    const DT_NUM: usize = 34;

    #[repr(C)]
    #[derive(Debug)]
    pub struct LinkMap {
        pub l_addr: Elf64_Addr,
        pub l_name: *const libc::c_char,
        pub l_ld: *const Elf64Dyn,
        pub l_next: *const LinkMap,
        pub l_prev: *const LinkMap,
        pub l_real: *const LinkMap,
        pub l_ns: usize,
        pub l_libname: *const libc::c_void,
        pub l_info: [*const Elf64Dyn; DT_NUM],
    }

    unsafe extern "C" fn dl_iterate_phdr_callback(
        info: *mut libc::dl_phdr_info,
        _size: usize,
        data: *mut std::ffi::c_void,
    ) -> i32 {
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
            dl_iterate_phdr(
                Some(dl_iterate_phdr_callback),
                (&mut info) as *mut libc::dl_phdr_info as *mut std::ffi::c_void,
            );

            // base addr is the offset to the real map from the vaddr in elf
            let base_addr = (info).dlpi_addr as usize;
            //eprintln!("Base addr {} (should be zero)", base_addr);

            //eprintln!("ph num = {}", info.dlpi_phnum);
            let phdr_slice = std::slice::from_raw_parts_mut(
                (info).dlpi_phdr as *mut Elf64_Phdr,
                (info).dlpi_phnum as usize,
            );
            // print all phdr
            /*for p in phdr_slice.iter() {
                eprintln!("Range: {:08x} -- {:08x} ", p.p_vaddr, p.p_vaddr + p.p_memsz);
                eprintln!("P_TYPE: {} (load = 1)", p.p_type);
            }*/
            let map_end = phdr_slice
                .iter()
                .filter(|p| p.p_type == PT_LOAD)
                .map(|p| p.p_vaddr + p.p_memsz)
                .max()
                .unwrap_or_default() as usize;
            let map_start = phdr_slice
                .iter()
                .filter(|p| p.p_type == PT_LOAD)
                .map(|p| p.p_vaddr)
                .min()
                .unwrap_or_default() as usize;

            let data = std::slice::from_raw_parts(
                (base_addr + map_start) as *const u8,
                map_end - map_start,
            );
            #[cfg(feature = "symbols")]
            let exe_path = std::fs::read_link("/proc/self/exe").ok();
            #[cfg(not(feature = "symbols"))]
            let exe_path: Option<std::path::PathBuf> = None;
            //eprintln!("Reading image internal");
            Image::read(Some(base_addr), data, exe_path, false)
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

    use crate::image::pe::PEImage;
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

        PEImage::read_inner_memory::<String>(image_base_address, None, false, memory, object)
    }
}
