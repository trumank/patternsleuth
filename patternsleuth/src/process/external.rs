#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "linux")]
mod linux {
    use std::ops::Range;

    use anyhow::{Context, Result, bail};
    use object::{Object, ObjectSection};

    use crate::{Image, Memory, image};

    fn read_process_mem(pid: i32, address: usize, buffer: &mut [u8]) -> Result<usize> {
        unsafe {
            let read = libc::process_vm_readv(
                pid as _,
                &libc::iovec {
                    iov_base: buffer.as_mut_ptr() as _,
                    iov_len: buffer.len(),
                },
                1,
                &libc::iovec {
                    iov_base: address as _,
                    iov_len: buffer.len(),
                },
                1,
                0,
            );

            if read == -1 {
                bail!("failed to read PID={pid} addr=0x{address:x}")
            }

            Ok(read as usize)
        }
    }

    /// Read `/proc/<PID>/maps` and find region ending with ".exe" which is the main module for
    /// processes running under WINE
    fn find_main_module(pid: i32) -> Result<Range<usize>> {
        let maps = std::fs::read_to_string(format!("/proc/{pid}/maps"))
            .with_context(|| format!("could not read process maps (PID={pid})"))?;
        for line in maps.lines() {
            let mut split = line.splitn(6, |c: char| c.is_whitespace());
            if let [
                Some(range),
                Some(_permissions),
                Some(_offset),
                Some(_device),
                Some(_inode),
                Some(path),
            ] = [
                split.next(),
                split.next(),
                split.next(),
                split.next(),
                split.next(),
                split.next(),
            ] {
                if path.ends_with(".exe") {
                    let (start, end) = range
                        .split_once('-')
                        .context("failed to parse map range: {range?}")?;
                    let range = usize::from_str_radix(start, 16)?..usize::from_str_radix(end, 16)?;
                    //println!("{range:x?} {permissions:?} {offset:?} {device:?} {inode:?} {:?}", path.trim_start());
                    return Ok(range);
                }
            } else {
                bail!("failed to parse line of maps: {line:?}");
            }
        }
        bail!("no main module found")
    }

    pub fn read_image_from_pid<'data>(pid: i32) -> Result<Image<'data>> {
        let main_module = find_main_module(pid)?;

        let mut image_header = vec![0; main_module.len()];

        read_process_mem(pid, main_module.start, &mut image_header)?;

        let object = object::File::parse(image_header.as_slice())?;

        let mut sections = vec![];
        for section in object.sections() {
            let mut data = vec![0; section.size() as usize];
            read_process_mem(pid, section.address() as usize, &mut data)?;
            sections.push((section, data));
        }

        let memory = Memory::new_external_data(sections)?;

        image::pe::PEImage::read_inner_memory::<String>(
            object.relative_address_base(),
            None,
            false,
            memory,
            object,
        )
    }
}

#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "macos")]
mod macos {
    use anyhow::Result;

    use crate::Image;

    pub fn read_image_from_pid<'data>(_pid: i32) -> Result<Image<'data>> {
        todo!()
    }
}

#[cfg(windows)]
pub use windows::*;

#[cfg(windows)]
mod windows {
    use anyhow::{Result, bail};
    use object::{Object, ObjectSection};

    use crate::image::pe::PEImage;
    use crate::{Image, Memory};

    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows::Win32::System::ProcessStatus::{
        EnumProcessModules, GetModuleInformation, MODULEINFO,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };

    pub fn read_image_from_pid<'data>(pid: i32) -> Result<Image<'data>> {
        let (memory, base) = unsafe {
            let process = OpenProcess(
                PROCESS_VM_READ | PROCESS_QUERY_INFORMATION,
                false,
                pid as u32,
            )?;

            let mut modules = [Default::default(); 1];
            let mut out_len = 0;
            EnumProcessModules(
                process,
                modules.as_mut_ptr(),
                (modules.len() * std::mem::size_of::<HMODULE>()) as u32,
                &mut out_len,
            )?;

            if out_len < 1 {
                bail!("expected at least one module");
            }

            let mut info = MODULEINFO::default();
            GetModuleInformation(
                process,
                modules[0],
                &mut info,
                std::mem::size_of::<MODULEINFO>() as u32,
            )?;

            let mut mem = vec![0u8; info.SizeOfImage as usize];
            ReadProcessMemory(
                process,
                info.lpBaseOfDll,
                mem.as_mut_ptr() as *mut std::ffi::c_void,
                mem.len(),
                None,
            )?;

            (mem, info.lpBaseOfDll as usize)
        };

        let object = object::File::parse(memory.as_slice())?;

        let mut sections = vec![];
        for section in object.sections() {
            let mut data = vec![0; section.size() as usize];
            let start = section.address() as usize - object.relative_address_base() as usize;
            let end = section.size() as usize + start;
            // TODO avoid this copy and re-allocation
            data.copy_from_slice(&memory[start..end]);
            sections.push((section, data));
        }

        let memory = Memory::new_external_data(sections)?;

        PEImage::read_inner_memory::<String>(base, None, false, memory, object)
    }
}
