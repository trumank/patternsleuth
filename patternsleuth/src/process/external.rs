#[cfg(unix)]
pub use unix::*;

#[cfg(unix)]
mod unix {
    use std::ops::Range;

    use anyhow::{bail, Context, Result};
    use object::{Object, ObjectSection};

    use crate::{Image, Memory};

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
            let mut split = line.split_whitespace();
            if let (
                Some(range),
                Some(_permissions),
                Some(_offset),
                Some(_device),
                Some(_inode),
                Some(path),
            ) = (
                split.next(),
                split.next(),
                split.next(),
                split.next(),
                split.next(),
                split.remainder(),
            ) {
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

        let base_address = object.relative_address_base() as usize;
        let exception_directory_range = match object {
            object::File::Pe64(ref inner) => {
                let exception_directory = inner
                    .data_directory(object::pe::IMAGE_DIRECTORY_ENTRY_EXCEPTION)
                    .context("no exception directory")?;

                let (address, size) = exception_directory.address_range();
                base_address + address as usize..base_address + (address + size) as usize
            }
            _ => 0..0,
        };

        let mut sections = vec![];
        for section in object.sections() {
            let mut data = vec![0; section.size() as usize];
            read_process_mem(pid, section.address() as usize, &mut data)?;
            sections.push((section, data));
        }

        let memory = Memory::new_external_data(sections)?;

        Ok(Image {
            base_address,
            exception_directory_range,
            exception_children_cache: Default::default(),
            memory,
            symbols: Default::default(),
        })
    }
}
