#[cfg(feature = "image-elf")]
pub mod elf;
mod macros;
#[cfg(feature = "image-pe")]
pub mod pe;

use crate::*;
use anyhow::Error;
#[cfg(feature = "image-elf")]
use elf::ElfImage;
use itertools::Itertools;
#[cfg(feature = "image-pe")]
use pe::PEImage;

use macros::*;

image_type_dispatch! {
    @enum ImageType as _image_type_reflection {
        PEImage(PEImage, "image-pe"),
        ElfImage(ElfImage, "image-elf"),
    }

    @fns {
        fn get_function(address: usize) -> Result<Option<RuntimeFunction>, MemoryAccessError>;
        fn get_root_function(address: usize) -> Result<Option<RuntimeFunction>, MemoryAccessError>;
        fn get_root_function_range(address: usize) -> Result<Option<Range<usize>>, MemoryAccessError>;
        fn get_child_functions(address: usize) -> Result<Vec<RuntimeFunction>, MemoryAccessError>;
        fn get_root_functions() -> Result<Vec<Range<usize>>, MemoryAccessError>;
    }
}

pub use _image_type_reflection as image_type_reflection;
use pe::PEImageBuilder;

pub struct Image<'data> {
    pub base_address: usize,
    pub memory: Box<dyn SectionedMemoryTrait<'data> + 'data>,
    #[cfg(feature = "symbols")]
    pub symbols: Option<HashMap<usize, symbols::Symbol>>,
    pub imports: HashMap<String, HashMap<String, usize>>,
    pub image_type: ImageType,
}

// Type-independent
impl<'data> Image<'data> {
    #[allow(unused)]
    pub fn read<P: AsRef<Path>>(
        base_addr: Option<usize>,
        data: &'data [u8],
        exe_path: Option<P>,
        cache_functions: bool,
    ) -> Result<Image<'data>> {
        let object = match object::File::parse(data) {
            Err(e) if e.to_string() == "Unknown file magic" => {
                fn merge_adjacent_slices<'a, T>(a: &'a [T], b: &'a [T]) -> &'a [T] {
                    assert_eq!(
                        unsafe { a.as_ptr().add(a.len()) },
                        b.as_ptr(),
                        "Slices are not adjacent in memory"
                    );

                    unsafe { std::slice::from_raw_parts(a.as_ptr(), a.len() + b.len()) }
                }

                let dump = minidump::Minidump::read(data)?;

                let mem = dump.get_memory().unwrap();
                let modules = dump.get_stream::<minidump::MinidumpModuleList>()?;
                let main = modules.main_module().unwrap();

                //let mut bytes: Option<&'data [u8]> = None;
                let base_address = main.raw.base_of_image;
                let size = main.raw.size_of_image;
                let main_range = base_address..(base_address + size as u64);

                // verify sorted by address
                for (a, b) in mem.by_addr().tuple_windows() {
                    assert!(a.base_address() < b.base_address());
                }

                let mut sections = vec![];
                let mut main_bytes = vec![0; size as usize];

                let mut chunk: Option<(&[u8], u64)> = None;

                for mem in mem.by_addr() {
                    let bytes = unsafe { std::mem::transmute::<&[u8], &'data [u8]>(mem.bytes()) };
                    if main_range.contains(&mem.base_address()) {
                        //println!("XX {:X} {:X}", mem.base_address(), mem.size());
                        let start = mem.base_address() - base_address;
                        main_bytes[start as usize..(start + mem.size()) as usize]
                            .copy_from_slice(bytes);
                    }

                    if let Some((slice, address)) = chunk {
                        // check if continuous with existing slice
                        if address + slice.len() as u64 == mem.base_address() {
                            // extend existing slice
                            chunk = Some((merge_adjacent_slices(slice, bytes), address));
                        } else {
                            // add section
                            sections.push(NamedMemorySection::new(
                                "".to_string(),
                                address as usize,
                                SectionFlags::default(),
                                slice,
                            ));
                            // reset slice
                            chunk = Some((bytes, mem.base_address()));
                        }
                    } else {
                        // set slice
                        chunk = Some((bytes, mem.base_address()));
                    }

                    //println!("{inc} {:X} {:X}", mem.base_address(), mem.size());
                }
                // add any remaining section
                if let Some((slice, address)) = chunk {
                    sections.push(NamedMemorySection::new(
                        "".to_string(),
                        address as usize,
                        SectionFlags::default(),
                        slice,
                    ));
                }

                let object = object::File::parse(main_bytes.as_slice())?;

                match object {
                    object::File::Pe64(_) => {
                        return PEImageBuilder::new()
                            .object(object)?
                            .memory(Box::new(Memory::new_sections(sections)))
                            .exe_path(exe_path.as_ref().map(AsRef::as_ref))
                            .build();
                    }
                    _ => unreachable!(),
                }
            }
            Err(e) => return Err(e.into()),
            Ok(o) => o,
        };

        match object {
            #[cfg(feature = "image-elf")]
            object::File::Elf64(_) => {
                ElfImage::read_inner(base_addr, exe_path, cache_functions, object)
            }
            #[cfg(feature = "image-pe")]
            object::File::Pe64(_) => PEImageBuilder::new()
                .memory_from_object(object)?
                .exe_path(exe_path.as_ref().map(AsRef::as_ref))
                .build(),
            _ => Err(Error::msg("Unsupported file format")),
        }
    }
    pub fn builder() -> ImageBuilder {
        Default::default()
    }

    pub fn scan<'patterns, S>(
        &self,
        pattern_configs: &'patterns [PatternConfig<S>],
    ) -> Result<ScanResult<'patterns, S>> {
        let mut results = vec![];

        struct PendingScan {
            original_config_index: usize,
            scan: Scan,
        }

        let scan_queue = pattern_configs
            .iter()
            .enumerate()
            .map(|(index, config)| PendingScan {
                original_config_index: index,
                scan: config.scan.clone(), // TODO clone isn't ideal but makes handling multi-stage scans a lot easier
            })
            .collect::<Vec<_>>();

        for section in self.memory.sections() {
            let base_address = section.address();
            let data = section.data();

            let (pattern_scans, patterns): (Vec<_>, Vec<_>) = scan_queue
                .iter()
                .filter_map(|scan| {
                    scan.scan
                        .scan_type
                        .get_pattern()
                        .map(|pattern| (scan, pattern))
                })
                .unzip();

            let (xref_scans, xrefs): (Vec<_>, Vec<_>) = scan_queue
                .iter()
                .filter_map(|scan| scan.scan.scan_type.get_xref().map(|xref| (scan, xref)))
                .unzip();

            let scan_results = scanner::scan_pattern(&patterns, base_address, data)
                .into_iter()
                .chain(scanner::scan_xref(&xrefs, base_address, data))
                .zip(pattern_scans.iter().chain(xref_scans.iter()));

            for (addresses, scan) in scan_results {
                for address in addresses {
                    results.push((
                        &pattern_configs[scan.original_config_index],
                        Resolution { address },
                    ));
                }
            }
        }

        Ok(ScanResult { results })
    }
}

#[derive(Default)]
pub struct ImageBuilder {
    functions: bool,
}
pub struct ImageBuilderWithSymbols<P: AsRef<Path>> {
    symbols: Option<P>,
    functions: bool,
}
impl ImageBuilder {
    pub fn functions(mut self, functions: bool) -> Self {
        self.functions = functions;
        self
    }
    #[cfg(feature = "symbols")]
    pub fn symbols<P: AsRef<Path>>(self, exe_path: P) -> ImageBuilderWithSymbols<P> {
        ImageBuilderWithSymbols {
            symbols: Some(exe_path),
            functions: self.functions,
        }
    }
    pub fn build(self, data: &[u8]) -> Result<Image<'_>> {
        Image::read::<&str>(None, data, None, self.functions)
    }
}
impl<P: AsRef<Path>> ImageBuilderWithSymbols<P> {
    pub fn functions(mut self, functions: bool) -> Self {
        self.functions = functions;
        self
    }
    #[cfg(feature = "symbols")]
    pub fn symbols(mut self, exe_path: P) -> Self {
        self.symbols = Some(exe_path);
        self
    }
    pub fn build(self, data: &[u8]) -> Result<Image<'_>> {
        Image::read(None, data, self.symbols, self.functions)
    }
}
