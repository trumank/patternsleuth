#[cfg(unix)]
pub use unix::*;

#[cfg(unix)]
mod unix {
    use anyhow::Result;

    use crate::Image;

    pub fn read_image<'data>() -> Result<Image<'data>> {
        todo!();
    }
}

#[cfg(windows)]
pub use windows::*;

#[cfg(windows)]
mod windows {
    use anyhow::{Context, Result};
    use object::{Object, ObjectSection};
    use windows::
        Win32::System::{
            LibraryLoader::GetModuleHandleA,
            ProcessStatus::{GetModuleInformation, MODULEINFO},
            Threading::GetCurrentProcess,
    };

    use crate::{Image, Memory};

    pub fn read_image<'data>() -> Result<Image<'data>> {
        let main_module = unsafe { GetModuleHandleA(None) }.context("could not find main module")?;
        let process = unsafe { GetCurrentProcess() };

        let mut mod_info = MODULEINFO::default();
            unsafe {
            GetModuleInformation(
                process,
                main_module,
                &mut mod_info as *mut _,
                std::mem::size_of::<MODULEINFO>() as u32,
            )
        };

        let module_addr = mod_info.lpBaseOfDll as usize;

        let memory = unsafe {
            std::slice::from_raw_parts(
                mod_info.lpBaseOfDll as *mut u8,
                mod_info.SizeOfImage as usize,
            )
        };

        let object = object::File::parse(memory)?;

        let exception_directory_range = match object {
            object::File::Pe64(ref inner) => {
                let exception_directory = inner
                    .data_directory(object::pe::IMAGE_DIRECTORY_ENTRY_EXCEPTION)
                    .context("no exception directory")?;

                let (address, size) = exception_directory.address_range();
                module_addr + address as usize..module_addr + (address + size) as usize
            }
            _ => 0..0,
        };

        let image_base_address = object.relative_address_base() as usize;

        let mut sections = vec![];
        for section in object.sections() {
            let addr = section.address() as usize - image_base_address as usize;
            let size = section.size() as usize;
            sections.push((section, &memory[addr..addr + size]));
        }

        let memory = Memory::new_internal_data(sections)?;

        Ok(Image {
            base_address: module_addr,
            exception_directory_range,
            exception_children_cache: Default::default(),
            memory,
            symbols: Default::default(),
        })
    }
}
