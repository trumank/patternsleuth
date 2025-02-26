use std::collections::{HashMap, HashSet};
use std::ops::Range;

use anyhow::{Context, Result, bail};
use itertools::Itertools;

use super::{Image, ImageType};
#[cfg(feature = "symbols")]
use crate::symbols;
use crate::{MemTraitNew, Memory, MemoryAccessError, RuntimeFunction};
use object::Object;

pub struct PEImage {
    pub exception_directory_range: Range<usize>,
    pub exception_children_cache: HashMap<usize, Vec<RuntimeFunction>>,
}

impl PEImage {
    pub fn get_function(
        &self,
        image: &Image<'_>,
        address: usize,
    ) -> Result<Option<RuntimeFunction>, MemoryAccessError> {
        // place holder only
        let size = 12;
        let mut min = 0;
        let mut max = self.exception_directory_range.len() / size - 1;

        while min <= max {
            let i = (max + min) / 2;
            let addr = i * size + self.exception_directory_range.start;

            let addr_begin = image.base_address + image.memory.u32_le(addr)? as usize;
            if addr_begin <= address {
                let addr_end = image.base_address + image.memory.u32_le(addr + 4)? as usize;
                if addr_end > address {
                    let unwind = image.base_address + image.memory.u32_le(addr + 8)? as usize;

                    return Ok(Some(RuntimeFunction {
                        range: addr_begin..addr_end,
                        unwind,
                    }));
                } else {
                    min = i + 1;
                }
            } else {
                max = i - 1;
            }
        }
        Ok(None)
    }
    pub fn get_root_function(
        &self,
        image: &Image<'_>,
        address: usize,
    ) -> Result<Option<RuntimeFunction>, MemoryAccessError> {
        if let Some(f) = self.get_function(image, address)? {
            let mut f = RuntimeFunction {
                range: f.range,
                unwind: f.unwind,
            };

            loop {
                let mut unwind_addr = f.unwind;

                let section = image.memory.get_section_containing(unwind_addr)?;

                let has_chain_info = section.index(unwind_addr)? >> 3 == 0x4;
                if has_chain_info {
                    let unwind_code_count = section.index(unwind_addr + 2)?;

                    unwind_addr += 4 + 2 * unwind_code_count as usize;
                    if unwind_addr % 4 != 0 {
                        // align
                        unwind_addr += 2;
                    }

                    if section.address() + section.data().len() > unwind_addr + 12 {
                        f = RuntimeFunction::read(section, image.base_address, unwind_addr)?;
                    } else {
                        todo!("not adding chain info {unwind_addr}");
                    }
                } else {
                    return Ok(Some(f));
                }
            }
        } else {
            Ok(None)
        }
    }

    pub fn get_root_function_range(
        &self,
        image: &Image<'_>,
        address: usize,
    ) -> Result<Option<Range<usize>>, MemoryAccessError> {
        let exception = self.get_root_function(image, address)?;
        if let Some(exception) = exception {
            let fns = self
                .get_child_functions(image, exception.range.start)
                .unwrap();
            let min = fns.iter().map(|f| f.range.start).min().unwrap();
            let max = fns.iter().map(|f| f.range.end).max().unwrap();
            if exception.range.start != address {
                // why comapreing exception but not min?
                Err(MemoryAccessError::MisalginedAddress(
                    exception.range.start,
                    address,
                ))
            } else {
                Ok(Some(min..max)) // TODO does not handle sparse ranges
            }
        } else {
            Ok(None)
        }
    }

    pub fn get_child_functions(
        &self,
        image: &Image<'_>,
        address: usize,
    ) -> Result<Vec<RuntimeFunction>, MemoryAccessError> {
        let mut queue = vec![address];
        let mut all_children = vec![self.get_function(image, address)?.unwrap()];
        while let Some(next) = queue.pop() {
            if let Some(children) = self.exception_children_cache.get(&next) {
                for child in children {
                    queue.push(child.range().start);
                    all_children.push(child.clone());
                }
            }
        }
        Ok(all_children)
    }

    pub fn get_root_functions(
        &self,
        image: &Image<'_>,
    ) -> Result<Vec<Range<usize>>, MemoryAccessError> {
        let mut functions = self.exception_children_cache.keys().collect::<HashSet<_>>();
        for e in self.exception_children_cache.values() {
            for c in e {
                functions.remove(&c.range.start);
            }
        }
        functions
            .iter()
            .map(|function| -> Result<Range<usize>, MemoryAccessError> {
                let fns = self
                    .get_child_functions(
                        image,
                        self.get_function(image, **function)?
                            .ok_or(MemoryAccessError::MemoryOutOfBoundsError)?
                            .range
                            .start,
                    )
                    .unwrap();
                let min = fns.iter().map(|f| f.range.start).min().unwrap();
                let max = fns.iter().map(|f| f.range.end).max().unwrap();
                Ok(min..max)
            })
            .try_collect()
    }
}

impl Image<'_> {
    // this function is privately used by pe image
    fn populate_exception_cache(&mut self) -> Result<(), MemoryAccessError> {
        #[allow(irrefutable_let_patterns)]
        if let ImageType::PEImage(ref mut pe) = self.image_type {
            for i in pe.exception_directory_range.clone().step_by(12) {
                let f = RuntimeFunction::read(self.memory.as_ref(), self.base_address, i)?;
                pe.exception_children_cache.insert(f.range.start, vec![]);

                let Ok(section) = self.memory.get_section_containing(f.unwind) else {
                    // TODO disabled cause spammy
                    //println!("invalid unwind info addr {:x}", f.unwind);
                    continue;
                };

                let mut unwind = f.unwind;
                let has_chain_info = section.index(unwind)? >> 3 == 0x4;
                if has_chain_info {
                    let unwind_code_count = section.index(unwind + 2)?;

                    unwind += 4 + 2 * unwind_code_count as usize;
                    if unwind % 4 != 0 {
                        // align
                        unwind += 2;
                    }

                    if section.address() + section.data().len() > unwind + 12 {
                        let chained = RuntimeFunction::read(section, self.base_address, unwind)?;

                        // TODO disabled because it spams the log too much
                        //let referenced = self.get_function(chained.range.start);

                        //assert_eq!(Some(&chained), referenced.as_ref());
                        //if Some(&chained) != referenced.as_ref() {
                        //println!("mismatch {:x?} {referenced:x?}", Some(&chained));
                        //}

                        pe.exception_children_cache
                            .entry(chained.range.start)
                            .or_default()
                            .push(f);
                    } else {
                        println!("invalid unwind addr {:x}", unwind);
                    }
                }
            }

            //println!("{:#x?}", self.exception_children_cache);
            Ok(())
        } else {
            unreachable!("not a PE image")
        }
    }
}

impl PEImage {
    /// Read and parse ELF object, using data from memory
    pub fn read_inner_memory<'data, P: AsRef<std::path::Path>>(
        base_address: usize,
        #[allow(unused_variables)] exe_path: Option<P>,
        cache_functions: bool,
        memory: Box<dyn MemTraitNew<'data> + 'data>,
        object: object::File<'_>,
    ) -> Result<Image<'data>, anyhow::Error> {
        #[cfg(feature = "symbols")]
        let symbols = if let Some(exe_path) = exe_path {
            let pdb_path = exe_path.as_ref().with_extension("pdb");
            pdb_path
                .exists()
                .then(|| symbols::dump_pdb_symbols(pdb_path, base_address))
                .transpose()?
        } else {
            None
        };

        let get_ex_dir = || -> Result<Range<usize>> {
            Ok(match object {
                object::File::Pe64(ref inner) => {
                    let exception_directory = inner
                        .data_directory(object::pe::IMAGE_DIRECTORY_ENTRY_EXCEPTION)
                        .context("no exception directory")?;

                    let (address, size) = exception_directory.address_range();
                    base_address + address as usize..base_address + (address + size) as usize
                }
                _ => bail!("not a PE file"),
            })
        };

        let get_imports = || -> Result<_> {
            Ok(match object {
                object::File::Pe64(ref inner) => {
                    use object::LittleEndian as LE;
                    use object::pe::ImageNtHeaders64;
                    use object::read::pe::ImageThunkData;

                    let mut imports: HashMap<String, HashMap<String, usize>> = Default::default();

                    let import_table = inner.import_table()?.unwrap();
                    let mut import_descs = import_table.descriptors()?;

                    while let Some(import_desc) = import_descs.next()? {
                        let mut cur = HashMap::new();

                        let Ok(lib_name) = import_table.name(import_desc.name.get(LE)) else {
                            continue;
                        };
                        let lib_name = std::str::from_utf8(lib_name)?.to_ascii_lowercase();
                        let mut thunks =
                            import_table.thunks(import_desc.original_first_thunk.get(LE))?;
                        let mut address = base_address + import_desc.first_thunk.get(LE) as usize;
                        while let Some(thunk) = thunks.next::<ImageNtHeaders64>()? {
                            if let Ok((_hint, name)) = import_table.hint_name(thunk.address()) {
                                cur.insert(std::str::from_utf8(name)?.to_owned(), address);
                                address += 8;
                            }
                        }
                        imports.insert(lib_name, cur);
                    }
                    imports
                }
                _ => bail!("not a PE file"),
            })
        };

        let mut new = Image {
            base_address,
            memory,
            #[cfg(feature = "symbols")]
            symbols,
            imports: get_imports().unwrap_or_default(),
            image_type: ImageType::PEImage(PEImage {
                exception_directory_range: get_ex_dir().unwrap_or_default(),
                exception_children_cache: Default::default(),
            }),
        };

        if cache_functions {
            new.populate_exception_cache()?;
        }
        Ok(new)
    }

    pub fn read_inner<P: AsRef<std::path::Path>>(
        base_addr: Option<usize>,
        exe_path: Option<P>,
        cache_functions: bool,
        object: object::File<'_>,
    ) -> Result<Image<'_>, anyhow::Error> {
        let base_address = base_addr.unwrap_or(object.relative_address_base() as usize);
        let memory = Memory::new(&object)?;
        Self::read_inner_memory(
            base_address,
            exe_path,
            cache_functions,
            Box::new(memory),
            object,
        )
    }
}
